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
    crdt_clock_scan_reservation, crdt_envelope, execute_mutation_plan, preflight_mutation_plan,
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
fn atom_only_textblock_supports_both_gaps_and_adjacent_opaque_atoms() {
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
}

#[test]
fn structural_delete_removes_an_inline_atom_directly() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "hardBreak" }]
        }]
    });

    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![TypedOperation::DeleteRange {
            range: range_for_test(0, 1),
        }],
    );

    assert_eq!(actual, expected);
    assert_eq!(
        actual,
        json!({ "type": "doc", "content": [{ "type": "paragraph" }] })
    );
}

#[test]
fn duplicate_equal_atom_deletes_preserve_the_identity_selected_by_operation_intent() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "hardBreak" }, { "type": "hardBreak" }]
        }]
    });

    let remaining_after_delete = |from, to| {
        let (doc, schema, limits, editing_limits, document) = diagnostic_doc(&source);
        let before_ids = {
            let txn = doc.transact();
            let fragment = txn.get_xml_fragment("prosemirror").unwrap();
            let XmlOut::Element(paragraph) = fragment.get(&txn, 0).unwrap() else {
                panic!("expected paragraph")
            };
            paragraph
                .children(&txn)
                .map(|child| child.id())
                .collect::<Vec<_>>()
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
                    request_id: 108,
                    base_document_revision: 0,
                    origin: TransactionOrigin::LocalInput,
                    operations: vec![TypedOperation::DeleteRange {
                        range: range_for_test(from, to),
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
            execute_mutation_plan(compiled.mutation_plan, &mut txn);
        }
        let remaining_id = {
            let txn = doc.transact();
            let fragment = txn.get_xml_fragment("prosemirror").unwrap();
            let XmlOut::Element(paragraph) = fragment.get(&txn, 0).unwrap() else {
                panic!("expected paragraph")
            };
            paragraph.get(&txn, 0).unwrap().id()
        };
        (before_ids, remaining_id)
    };

    let (first_case_ids, after_delete_first) = remaining_after_delete(0, 1);
    assert_eq!(after_delete_first, first_case_ids[1]);
    let (second_case_ids, after_delete_second) = remaining_after_delete(1, 2);
    assert_eq!(after_delete_second, second_case_ids[0]);
}

#[test]
fn duplicate_equal_atom_inserts_preserve_existing_identities_before_and_after() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "hardBreak" }, { "type": "hardBreak" }]
        }]
    });

    let insert_and_ids = |at| {
        let (doc, schema, limits, editing_limits, document) = diagnostic_doc(&source);
        let original_ids = {
            let txn = doc.transact();
            let fragment = txn.get_xml_fragment("prosemirror").unwrap();
            let XmlOut::Element(paragraph) = fragment.get(&txn, 0).unwrap() else {
                panic!("expected paragraph")
            };
            paragraph
                .children(&txn)
                .map(|child| child.id())
                .collect::<Vec<_>>()
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
                    request_id: 109,
                    base_document_revision: 0,
                    origin: TransactionOrigin::LocalInput,
                    operations: vec![TypedOperation::InsertNode {
                        at: point_for_test(at),
                        node: Node::void("hardBreak".into(), HashMap::new()),
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
            execute_mutation_plan(compiled.mutation_plan, &mut txn);
        }
        let after_ids = {
            let txn = doc.transact();
            let fragment = txn.get_xml_fragment("prosemirror").unwrap();
            let XmlOut::Element(paragraph) = fragment.get(&txn, 0).unwrap() else {
                panic!("expected paragraph")
            };
            paragraph
                .children(&txn)
                .map(|child| child.id())
                .collect::<Vec<_>>()
        };
        (original_ids, after_ids)
    };

    let (before_insert, after_insert_before) = insert_and_ids(0);
    assert_eq!(&after_insert_before[1..], before_insert.as_slice());
    let (before_append, after_insert_after) = insert_and_ids(2);
    assert_eq!(&after_insert_after[..2], before_append.as_slice());
}

#[test]
fn structural_insert_inside_text_retains_left_storage_and_creates_atom_and_suffix() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "abcd" }]
        }]
    });
    let (doc, schema, limits, editing_limits, document) = diagnostic_doc(&source);
    let left_id = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        <XmlTextRef as AsRef<Branch>>::as_ref(&paragraph_text(&fragment, &txn, 0)).id()
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
                request_id: 112,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertNode {
                    at: point_for_test(2),
                    node: Node::void("hardBreak".into(), HashMap::new()),
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
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let XmlOut::Element(paragraph) = fragment.get(&txn, 0).unwrap() else {
        panic!("expected paragraph")
    };
    let children = paragraph.children(&txn).collect::<Vec<_>>();
    assert_eq!(children.len(), 3);
    assert_eq!(children[0].id(), left_id);
    assert!(matches!(children[0], XmlOut::Text(_)));
    assert!(matches!(children[1], XmlOut::Element(_)));
    assert!(matches!(children[2], XmlOut::Text(_)));
    assert_eq!(
        YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap(),
        to_prosemirror_json(&compiled.preview, &schema)
    );
}

#[test]
fn structural_insert_preserves_unaffected_identity_and_supports_replica_undo_redo() {
    let source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "AB" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }
        ]
    });
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![TypedOperation::InsertNode {
            at: point_for_test(1),
            node: Node::void("hardBreak".into(), HashMap::new()),
        }],
        tiptap_schema(),
    );
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    let (before_update, tail_id, tail_text_id, sticky) = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let tail = fragment.get(&txn, 1).unwrap();
        let tail_text = paragraph_text(&fragment, &txn, 1);
        (
            txn.encode_state_as_update_v1(&StateVector::default()),
            tail.id(),
            <XmlTextRef as AsRef<Branch>>::as_ref(&tail_text).id(),
            StickyIndex::at(
                &txn,
                BranchPtr::from(<XmlTextRef as AsRef<Branch>>::as_ref(&tail_text)),
                2,
                Assoc::After,
            )
            .unwrap(),
        )
    };
    let fragment = doc.transact().get_xml_fragment("prosemirror").unwrap();
    let mut undo = UndoManager::<()>::new();
    undo.expand_scope(&doc, &fragment);
    undo.include_origin(TransactionOrigin::LocalCommand.as_yrs_origin());
    let update = {
        let mut txn = doc.transact_mut_with(TransactionOrigin::LocalCommand.as_yrs_origin());
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
        txn.commit();
        txn.encode_update_v1()
    };
    {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        assert_eq!(fragment.get(&txn, 1).unwrap().id(), tail_id);
        assert_eq!(
            <XmlTextRef as AsRef<Branch>>::as_ref(&paragraph_text(&fragment, &txn, 1)).id(),
            tail_text_id
        );
        assert_eq!(
            super::sticky_index_to_doc_pos(&txn, &fragment, &sticky, &schema),
            Some(8)
        );
        assert_eq!(
            YrsDocumentCodec::new(&schema, &limits)
                .read_json(&fragment, &txn)
                .unwrap(),
            expected
        );
    }
    let replica = utf16_doc();
    {
        let mut txn = replica.transact_mut();
        txn.apply_update(Update::decode_v1(&before_update).unwrap())
            .unwrap();
        txn.apply_update(Update::decode_v1(&update).unwrap())
            .unwrap();
    }
    {
        let txn = replica.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        assert_eq!(
            YrsDocumentCodec::new(&schema, &limits)
                .read_json(&fragment, &txn)
                .unwrap(),
            expected
        );
    }
    assert!(undo.undo_blocking());
    {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        assert_eq!(
            YrsDocumentCodec::new(&schema, &limits)
                .read_json(&fragment, &txn)
                .unwrap(),
            source
        );
    }
    assert!(undo.redo_blocking());
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    assert_eq!(
        YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap(),
        expected
    );
}

#[test]
fn insert_node_offset_node_class_and_recursive_attribute_matrix() {
    let unicode_source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "A😀B" }]
        }]
    });
    for (kind, offset) in [(EditorOffsetKind::Scalar, 2), (EditorOffsetKind::Utf16, 3)] {
        for affinity in [Affinity::Before, Affinity::After] {
            let (actual, expected, _, _, _) = compile_and_execute(
                unicode_source.clone(),
                vec![TypedOperation::InsertNode {
                    at: RevisionedPosition {
                        offset,
                        kind,
                        affinity,
                    },
                    node: Node::void("hardBreak".into(), HashMap::new()),
                }],
            );
            assert_eq!(actual, expected);
            assert_eq!(actual["content"][0]["content"][1]["type"], "hardBreak");
        }
    }

    let source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "A😀" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "B" }] }
        ]
    });
    let schema = attribute_schema();
    let scalar_at = rendered_scalar_offset(&source, &schema, "B") - 1;
    let image_attrs = HashMap::from([
        ("src".into(), Value::String("asset://direct".into())),
        ("alt".into(), Value::String("Direct image".into())),
    ]);
    let rich_quote = Node::element(
        "blockquote".into(),
        HashMap::new(),
        Fragment::from(vec![
            Node::element(
                "paragraph".into(),
                HashMap::new(),
                Fragment::from(vec![Node::text(
                    "quoted😀".into(),
                    vec![Mark::new("bold".into(), HashMap::new())],
                )]),
            ),
            Node::element(
                "taskList".into(),
                HashMap::from([("listMeta".into(), json!({ "owner": "team", "rank": 7 }))]),
                Fragment::from(vec![Node::element(
                    "taskItem".into(),
                    HashMap::from([
                        ("checked".into(), Value::Bool(true)),
                        (
                            "itemMeta".into(),
                            json!({ "id": "task-1", "flags": [1, false] }),
                        ),
                    ]),
                    Fragment::from(vec![
                        Node::element(
                            "paragraph".into(),
                            HashMap::new(),
                            Fragment::from(vec![Node::text("task".into(), vec![])]),
                        ),
                        Node::void(
                            "image".into(),
                            HashMap::from([
                                ("src".into(), Value::String("asset://nested".into())),
                                ("alt".into(), Value::String("Nested image".into())),
                            ]),
                        ),
                        Node::void(
                            "customBlock".into(),
                            HashMap::from([(
                                "meta".into(),
                                json!({ "nested": { "values": [1, "x", true] } }),
                            )]),
                        ),
                    ]),
                )]),
            ),
        ]),
    );

    for inserted in [Node::void("image".into(), image_attrs), rich_quote] {
        for (kind, offset) in [
            (EditorOffsetKind::Scalar, scalar_at),
            (EditorOffsetKind::Utf16, scalar_at + 1),
        ] {
            for affinity in [Affinity::Before, Affinity::After] {
                let (doc, schema, limits, compiled) = compile_operations_with_schema(
                    &source,
                    vec![TypedOperation::InsertNode {
                        at: RevisionedPosition {
                            offset,
                            kind,
                            affinity,
                        },
                        node: inserted.clone(),
                    }],
                    schema.clone(),
                );
                let expected = to_prosemirror_json(&compiled.preview, &schema);
                let before_update = doc
                    .transact()
                    .encode_state_as_update_v1(&StateVector::default());
                let update = {
                    let mut txn = doc.transact_mut();
                    execute_mutation_plan(compiled.mutation_plan, &mut txn);
                    txn.commit();
                    txn.encode_update_v1()
                };
                let txn = doc.transact();
                let fragment = txn.get_xml_fragment("prosemirror").unwrap();
                assert_eq!(
                    YrsDocumentCodec::new(&schema, &limits)
                        .read_json(&fragment, &txn)
                        .unwrap(),
                    expected
                );
                let replica = utf16_doc();
                {
                    let mut replica_txn = replica.transact_mut();
                    replica_txn
                        .apply_update(Update::decode_v1(&before_update).unwrap())
                        .unwrap();
                    replica_txn
                        .apply_update(Update::decode_v1(&update).unwrap())
                        .unwrap();
                }
                let replica_txn = replica.transact();
                let replica_fragment = replica_txn.get_xml_fragment("prosemirror").unwrap();
                assert_eq!(
                    YrsDocumentCodec::new(&schema, &limits)
                        .read_json(&replica_fragment, &replica_txn)
                        .unwrap(),
                    expected
                );
            }
        }
    }
}

#[test]
fn structural_local_origin_undo_redo_and_bound_matrix() {
    let atom_source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "hardBreak" }]
        }]
    });
    let split_source = json!({
        "type": "doc",
        "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "ab" }] }]
    });
    let join_source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "ab" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "cd" }] }
        ]
    });
    let wrap_source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "one" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }
        ]
    });
    let indent_source = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }] },
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }] }
            ]
        }]
    });
    let outdent_source = json!({
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
    });
    let unwrap_source = json!({
        "type": "doc",
        "content": [{ "type": "bulletList", "content": [{ "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }] }] }]
    });
    let schema = tiptap_schema();
    let cases = vec![
        (
            split_source.clone(),
            vec![TypedOperation::InsertNode {
                at: point_for_test(1),
                node: Node::void("hardBreak".into(), HashMap::new()),
            }],
        ),
        (
            atom_source.clone(),
            vec![TypedOperation::DeleteRange {
                range: range_for_test(0, 1),
            }],
        ),
        (
            atom_source,
            vec![TypedOperation::ReplaceRange {
                range: range_for_test(0, 1),
                content: Fragment::from(vec![Node::text("X".into(), vec![])]),
            }],
        ),
        (
            split_source,
            vec![TypedOperation::SplitBlock {
                at: point_for_test(1),
                node_type: "paragraph".into(),
                attrs: HashMap::new(),
            }],
        ),
        (
            join_source,
            vec![TypedOperation::JoinBlocks {
                at: point_for_test(2),
            }],
        ),
        (
            wrap_source,
            vec![TypedOperation::WrapInList {
                range: range_for_test(0, 3),
                list_type: "bulletList".into(),
                item_type: "listItem".into(),
                attrs: HashMap::new(),
                item_attrs: HashMap::new(),
            }],
        ),
        (
            indent_source.clone(),
            vec![TypedOperation::IndentListItem {
                at: point_for_test(rendered_scalar_offset(&indent_source, &schema, "two") + 1),
            }],
        ),
        (
            outdent_source.clone(),
            vec![TypedOperation::OutdentListItem {
                at: point_for_test(rendered_scalar_offset(&outdent_source, &schema, "inner") + 1),
            }],
        ),
        (
            unwrap_source.clone(),
            vec![TypedOperation::UnwrapFromList {
                at: point_for_test(rendered_scalar_offset(&unwrap_source, &schema, "one") + 1),
            }],
        ),
    ];

    for (case_index, (source, operations)) in cases.into_iter().enumerate() {
        let (doc, schema, limits, compiled) =
            compile_operations_with_schema(&source, operations, tiptap_schema());
        let expected = to_prosemirror_json(&compiled.preview, &schema);
        let undo_bound = compiled.undo_units_bound;
        let fragment = doc.transact().get_xml_fragment("prosemirror").unwrap();
        let mut undo = UndoManager::<()>::new();
        undo.expand_scope(&doc, &fragment);
        undo.include_origin(TransactionOrigin::LocalCommand.as_yrs_origin());
        {
            let mut txn = doc.transact_mut_with(TransactionOrigin::LocalCommand.as_yrs_origin());
            execute_mutation_plan(compiled.mutation_plan, &mut txn);
        }
        {
            let txn = doc.transact();
            let fragment = txn.get_xml_fragment("prosemirror").unwrap();
            assert_eq!(
                YrsDocumentCodec::new(&schema, &limits)
                    .read_json(&fragment, &txn)
                    .unwrap(),
                expected,
                "case {case_index} preview"
            );
        }
        let undo_item = undo
            .undo_stack()
            .last()
            .unwrap_or_else(|| panic!("case {case_index} was not captured by local origin"));
        let id_set_units = |set: &yrs::IdSet| {
            set.iter()
                .flat_map(|(_, ranges)| ranges.into_iter())
                .map(|range| u64::from(range.end - range.start))
                .sum::<u64>()
        };
        let actual_undo_units =
            id_set_units(undo_item.insertions()) + id_set_units(undo_item.deletions());
        assert!(actual_undo_units <= undo_bound, "case {case_index}");
        assert!(undo.undo_blocking(), "case {case_index} undo");
        {
            let txn = doc.transact();
            let fragment = txn.get_xml_fragment("prosemirror").unwrap();
            assert_eq!(
                YrsDocumentCodec::new(&schema, &limits)
                    .read_json(&fragment, &txn)
                    .unwrap(),
                source,
                "case {case_index} source"
            );
        }
        assert!(undo.redo_blocking(), "case {case_index} redo");
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        assert_eq!(
            YrsDocumentCodec::new(&schema, &limits)
                .read_json(&fragment, &txn)
                .unwrap(),
            expected,
            "case {case_index} redo preview"
        );
    }

    let source = json!({
        "type": "doc",
        "content": [{ "type": "image", "attrs": { "src": "old", "alt": "old alt" } }]
    });
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![TypedOperation::UpdateNodeAttrs {
            at: point_for_test(0),
            attrs: HashMap::from([
                ("src".into(), Value::String("new".into())),
                ("alt".into(), Value::String("new alt".into())),
            ]),
        }],
        attribute_schema(),
    );
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    let undo_bound = compiled.undo_units_bound;
    let fragment = doc.transact().get_xml_fragment("prosemirror").unwrap();
    let mut undo = UndoManager::<()>::new();
    undo.expand_scope(&doc, &fragment);
    undo.include_origin(TransactionOrigin::LocalCommand.as_yrs_origin());
    {
        let mut txn = doc.transact_mut_with(TransactionOrigin::LocalCommand.as_yrs_origin());
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let item = undo.undo_stack().last().expect("attribute update captured");
    let id_set_units = |set: &yrs::IdSet| {
        set.iter()
            .flat_map(|(_, ranges)| ranges.into_iter())
            .map(|range| u64::from(range.end - range.start))
            .sum::<u64>()
    };
    assert!(id_set_units(item.insertions()) + id_set_units(item.deletions()) <= undo_bound);
    assert!(undo.undo_blocking());
    {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        assert_eq!(
            YrsDocumentCodec::new(&schema, &limits)
                .read_json(&fragment, &txn)
                .unwrap(),
            source
        );
    }
    assert!(undo.redo_blocking());
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    assert_eq!(
        YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap(),
        expected
    );
}

#[test]
fn insert_split_join_and_attribute_undo_limits_are_exact() {
    fn assert_exact(
        source: &Value,
        operations: Vec<TypedOperation>,
        schema: crate::schema::Schema,
    ) {
        let exact = compile_operations_with_undo_limit(
            source,
            operations.clone(),
            schema.clone(),
            u64::MAX,
        )
        .unwrap()
        .undo_units_bound;
        assert!(exact > 0);
        assert_eq!(
            compile_operations_with_undo_limit(source, operations.clone(), schema.clone(), exact,)
                .unwrap()
                .undo_units_bound,
            exact
        );
        let error =
            compile_operations_with_undo_limit(source, operations, schema, exact - 1).unwrap_err();
        assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED");
        assert_eq!(error.limit, Some(exact - 1));
        assert_eq!(error.actual, Some(exact));
    }

    let paragraph = json!({
        "type": "doc",
        "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "A😀B" }] }]
    });
    assert_exact(
        &paragraph,
        vec![TypedOperation::InsertNode {
            at: point_for_test(2),
            node: Node::void("hardBreak".into(), HashMap::new()),
        }],
        tiptap_schema(),
    );
    assert_exact(
        &paragraph,
        vec![TypedOperation::SplitBlock {
            at: point_for_test(2),
            node_type: "paragraph".into(),
            attrs: HashMap::new(),
        }],
        tiptap_schema(),
    );
    let join = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "ab" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "cd" }] }
        ]
    });
    assert_exact(
        &join,
        vec![TypedOperation::JoinBlocks {
            at: point_for_test(2),
        }],
        tiptap_schema(),
    );
    let image = json!({
        "type": "doc",
        "content": [{ "type": "image", "attrs": { "src": "old", "alt": "old alt" } }]
    });
    assert_exact(
        &image,
        vec![TypedOperation::UpdateNodeAttrs {
            at: point_for_test(0),
            attrs: HashMap::from([
                ("src".into(), Value::String("new".into())),
                ("alt".into(), Value::String("new alt".into())),
            ]),
        }],
        attribute_schema(),
    );
}

#[test]
fn undo_limit_error_attributes_the_crossing_operation_before_a_trailing_noop() {
    let source = json!({
        "type": "doc",
        "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "AB" }] }]
    });
    let error = compile_operations_with_undo_limit(
        &source,
        vec![
            TypedOperation::InsertText {
                at: point_for_test(1),
                text: "XY".into(),
                marks: vec![],
            },
            TypedOperation::AddMark {
                range: range_for_test(0, 0),
                mark: Mark::new("bold".into(), HashMap::new()),
            },
        ],
        tiptap_schema(),
        1,
    )
    .unwrap_err();
    assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(error.operation_index, Some(0));
    assert_eq!(error.limit, Some(1));
    assert_eq!(error.actual, Some(2));
    assert_eq!(
        error.details,
        Some(json!({ "field": "maxUndoRetainedUnits" }))
    );
}

#[test]
fn block_insert_node_at_rendered_inter_block_break_targets_the_root_boundary() {
    let source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "A" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "B" }] }
        ]
    });
    let schema = tiptap_schema();
    let break_offset = rendered_scalar_offset(&source, &schema, "B") - 1;
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![TypedOperation::InsertNode {
            at: point_for_test(break_offset),
            node: Node::void("horizontalRule".into(), HashMap::new()),
        }],
    );
    assert_eq!(actual, expected);
    assert_eq!(
        actual["content"]
            .as_array()
            .unwrap()
            .iter()
            .map(|node| node["type"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["paragraph", "horizontalRule", "paragraph"]
    );
}

#[test]
fn wide_block_insert_resolver_admits_exact_work_and_rejects_one_under_atomically() {
    let large_text = format!("{}😀", "A".repeat(4_096));
    let mut wide_inline = vec![json!({ "type": "text", "text": large_text })];
    wide_inline.extend(
        (0..160)
            .map(|_| json!({ "type": "hardBreak" }))
            .collect::<Vec<_>>(),
    );
    wide_inline.push(json!({ "type": "text", "text": "end" }));
    let source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": wide_inline },
            { "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }
        ]
    });
    let (doc, schema, limits, editing_limits, document) = diagnostic_doc(&source);
    let at = rendered_scalar_offset(&source, &schema, "tail") - 1;
    let compile = |resource_limits: &ResourceLimits| {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        compile_transaction_with_yrs(
            CompilationContext {
                document: &document,
                selection: None,
                schema: &schema,
                resource_limits,
                editing_limits: &editing_limits,
                document_revision: 0,
                max_length: None,
            },
            TypedTransaction {
                request_id: 116,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalCommand,
                operations: vec![TypedOperation::InsertNode {
                    at: point_for_test(at),
                    node: Node::void("horizontalRule".into(), HashMap::new()),
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
    };

    let baseline = compile(&limits).unwrap();
    let resolver_work = baseline.mutation_plan.position_resolver_work_for_test();
    assert!(resolver_work > 4_256);
    let exact = baseline.mutation_plan.scan_work;
    let mut exact_limits = limits.clone();
    exact_limits.max_input_bytes = exact;
    let admitted = compile(&exact_limits).unwrap();
    assert_eq!(admitted.mutation_plan.scan_work, exact);
    assert_eq!(
        admitted.mutation_plan.position_resolver_work_for_test(),
        resolver_work
    );

    let txn = doc.transact();
    let before = txn.encode_state_as_update_v1(&StateVector::default());
    drop(txn);
    exact_limits.max_input_bytes = exact - 1;
    let error = compile(&exact_limits).unwrap_err();
    assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(u64::try_from(exact - 1).unwrap()));
    assert_eq!(error.actual, Some(u64::try_from(exact).unwrap()));
    let txn = doc.transact();
    assert_eq!(
        txn.encode_state_as_update_v1(&StateVector::default()),
        before
    );
    drop(txn);

    let non_resolver_work = exact.checked_sub(resolver_work).unwrap();
    let early_limit = non_resolver_work.checked_add(20).unwrap();
    assert!(early_limit < exact);
    exact_limits.max_input_bytes = early_limit;
    let error = compile(&exact_limits).unwrap_err();
    assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(u64::try_from(early_limit).unwrap()));
    assert_eq!(error.actual, Some(u64::try_from(early_limit + 1).unwrap()));
    let txn = doc.transact();
    assert_eq!(
        txn.encode_state_as_update_v1(&StateVector::default()),
        before
    );
}

#[test]
fn opaque_block_insert_at_rendered_break_targets_root_and_preserves_wire_tree() {
    let source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "A" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "B" }] }
        ]
    });
    let original = json!({
        "type": "mysteryBlock",
        "attrs": { "payload": [1, 2, 3] },
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "wire-only" }]
        }]
    });
    let opaque = Node::void(
        "__opaque_json".into(),
        HashMap::from([
            ("original_type".into(), Value::String("mysteryBlock".into())),
            ("original_json".into(), original.clone()),
            ("opaque_placement".into(), Value::String("block".into())),
        ]),
    );
    let schema = tiptap_schema();
    let break_offset = rendered_scalar_offset(&source, &schema, "B") - 1;
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![TypedOperation::InsertNode {
            at: point_for_test(break_offset),
            node: opaque,
        }],
    );
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][1], original);
    assert_eq!(actual["content"][2]["content"][0]["text"], "B");
}

#[test]
fn block_insert_node_maps_public_start_end_and_empty_block_boundaries() {
    for (source, offset, affinity, expected_index) in [
        (
            json!({
                "type": "doc",
                "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "A" }] }]
            }),
            0,
            Affinity::Before,
            0,
        ),
        (
            json!({
                "type": "doc",
                "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "A" }] }]
            }),
            1,
            Affinity::After,
            1,
        ),
        (
            json!({ "type": "doc", "content": [{ "type": "paragraph" }] }),
            0,
            Affinity::Before,
            0,
        ),
        (
            json!({ "type": "doc", "content": [{ "type": "paragraph" }] }),
            1,
            Affinity::After,
            1,
        ),
    ] {
        let (actual, expected, _, _, _) = compile_and_execute(
            source,
            vec![TypedOperation::InsertNode {
                at: RevisionedPosition {
                    offset,
                    kind: EditorOffsetKind::Scalar,
                    affinity,
                },
                node: Node::void("horizontalRule".into(), HashMap::new()),
            }],
        );
        assert_eq!(actual, expected);
        assert_eq!(actual["content"][expected_index]["type"], "horizontalRule");
    }
}

#[test]
fn inline_insert_node_keeps_textblock_mapping_at_public_start_and_end() {
    for (source, offset, affinity, expected_inline_index) in [
        (
            json!({
                "type": "doc",
                "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "A" }] }]
            }),
            0,
            Affinity::Before,
            0,
        ),
        (
            json!({
                "type": "doc",
                "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "A" }] }]
            }),
            1,
            Affinity::After,
            1,
        ),
    ] {
        let (actual, expected, _, _, _) = compile_and_execute(
            source,
            vec![TypedOperation::InsertNode {
                at: RevisionedPosition {
                    offset,
                    kind: EditorOffsetKind::Scalar,
                    affinity,
                },
                node: Node::void("hardBreak".into(), HashMap::new()),
            }],
        );
        assert_eq!(actual, expected);
        assert_eq!(actual["content"].as_array().unwrap().len(), 1);
        assert_eq!(
            actual["content"][0]["content"][expected_inline_index]["type"],
            "hardBreak"
        );
    }
}

#[test]
fn custom_inline_roles_preserve_every_offset_mapping_for_direct_insert_node() {
    let schema = Schema::from_json(&json!({
        "nodes": [
            { "name": "root", "content": "block*", "role": "doc" },
            { "name": "body", "content": "inline*", "group": "block", "role": "textBlock" },
            { "name": "softBreak", "content": "", "group": "inline", "role": "hardBreak", "isVoid": true, "allowUndeclaredAttrs": true },
            { "name": "widget", "content": "", "group": "inline", "role": "inline", "isVoid": true, "allowUndeclaredAttrs": true },
            { "name": "text", "group": "inline", "role": "text" }
        ],
        "marks": []
    }))
    .unwrap();
    let long_label = "😀".repeat(2_048);
    let source = json!({
        "type": "root",
        "content": [{
            "type": "body",
            "content": [
                { "type": "softBreak", "attrs": { "label": "ignored-long-label" } },
                { "type": "widget", "attrs": { "label": long_label.clone() } }
            ]
        }]
    });
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Error).unwrap();
    let rendered = crate::render::rendered_text(&document, &schema);
    let map = PositionMap::build(&document, &schema);
    assert_eq!(rendered, format!("\n[{long_label}]"));
    let terminal_scalar = 1 + 2 + u32::try_from(long_label.chars().count()).unwrap();
    let mapped = (0..=terminal_scalar)
        .map(|offset| map.scalar_to_doc(offset, &document))
        .collect::<Vec<_>>();
    assert_eq!(mapped[0], 1);
    assert!(mapped[1..terminal_scalar as usize]
        .iter()
        .all(|position| *position == 2));
    assert_eq!(mapped[terminal_scalar as usize], 3);
    assert_eq!(
        (0..=3)
            .map(|position| map.doc_to_scalar(position, &document))
            .collect::<Vec<_>>(),
        vec![0, 0, 1, terminal_scalar]
    );

    let limits = ResourceLimits::default();
    let editing_limits = EditingLimits::default();
    let doc = utf16_doc();
    let codec = YrsDocumentCodec::new(&schema, &limits);
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        codec
            .apply_json(&fragment, &mut txn, &json!({ "type": "root" }), &source)
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
                request_id: 117,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalCommand,
                operations: vec![TypedOperation::InsertNode {
                    at: RevisionedPosition {
                        offset: 2,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    node: Node::void(
                        "widget".into(),
                        HashMap::from([("label".into(), Value::String("Grace".into()))]),
                    ),
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
        .unwrap()
    };
    assert_eq!(
        to_prosemirror_json(&compiled.preview, &schema)["content"][0]["content"]
            .as_array()
            .unwrap()
            .iter()
            .map(|node| node["attrs"]["label"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["ignored-long-label", "Grace", long_label.as_str()]
    );
    assert!(compiled.mutation_plan.position_resolver_work_for_test() > long_label.len());
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    assert_eq!(
        codec.read_json(&fragment, &txn).unwrap(),
        to_prosemirror_json(&compiled.preview, &schema)
    );
}

#[test]
fn block_insert_at_separator_between_list_items_uses_an_affinity_valid_item_boundary() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }] },
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }] }
            ]
        }]
    });
    let schema = tiptap_schema();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&document, &schema);
    let separator =
        u32::try_from(rendered[..rendered.find('\n').unwrap()].chars().count()).unwrap();
    for (affinity, item_index) in [(Affinity::Before, 0usize), (Affinity::After, 1usize)] {
        let (actual, expected, _, _, _) = compile_and_execute(
            source.clone(),
            vec![TypedOperation::InsertNode {
                at: RevisionedPosition {
                    offset: separator,
                    kind: EditorOffsetKind::Scalar,
                    affinity,
                },
                node: Node::void("horizontalRule".into(), HashMap::new()),
            }],
        );
        assert_eq!(actual, expected);
        assert_eq!(
            actual["content"][0]["content"][item_index]["content"][1]["type"],
            "horizontalRule"
        );
    }
}

#[test]
fn split_block_then_block_insert_at_same_revisioned_position_uses_created_boundary() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "AB" }]
        }]
    });
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![
            TypedOperation::SplitBlock {
                at: point_for_test(1),
                node_type: "paragraph".into(),
                attrs: HashMap::new(),
            },
            TypedOperation::InsertNode {
                at: point_for_test(1),
                node: Node::void("horizontalRule".into(), HashMap::new()),
            },
        ],
    );
    assert_eq!(actual, expected);
    assert_eq!(
        actual["content"]
            .as_array()
            .unwrap()
            .iter()
            .map(|node| node["type"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["paragraph", "horizontalRule", "paragraph"]
    );
}

#[test]
fn nested_opaque_json_insert_remains_one_semantic_atom_for_follow_up_edits() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "ab" }]
        }]
    });
    let original = json!({
        "type": "mysteryInline",
        "attrs": { "payload": { "nested": true } },
        "content": [{ "type": "text", "text": "wire-only" }]
    });
    let opaque = Node::void(
        "__opaque_json".into(),
        HashMap::from([
            (
                "original_type".into(),
                Value::String("mysteryInline".into()),
            ),
            ("original_json".into(), original.clone()),
            ("opaque_placement".into(), Value::String("inline".into())),
        ]),
    );
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![
            TypedOperation::InsertNode {
                at: point_for_test(1),
                node: opaque,
            },
            TypedOperation::InsertText {
                at: point_for_test(1),
                text: "X".into(),
                marks: vec![],
            },
        ],
    );
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][0]["content"][1], original);
    assert_eq!(actual["content"][0]["content"][2]["text"], "Xb");
}

#[test]
fn existing_unknown_wire_element_with_descendants_has_void_semantic_size() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [
                { "type": "text", "text": "a" },
                {
                    "type": "mysteryInline",
                    "content": [{ "type": "text", "text": "wire-only" }]
                },
                { "type": "text", "text": "b" }
            ]
        }]
    });
    let schema = tiptap_schema();
    let b = rendered_scalar_offset(&source, &schema, "b");
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![TypedOperation::InsertText {
            at: point_for_test(b),
            text: "X".into(),
            marks: vec![],
        }],
    );
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][0]["content"][1]["type"], "mysteryInline");
    assert_eq!(actual["content"][0]["content"][2]["text"], "Xb");
}

#[test]
fn existing_unknown_block_wire_tree_is_one_semantic_atom_for_follow_up_text() {
    let source = json!({
        "type": "doc",
        "content": [
            {
                "type": "mysteryBlock",
                "content": [{
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": "wire-only" }]
                }]
            },
            { "type": "paragraph", "content": [{ "type": "text", "text": "B" }] }
        ]
    });
    let schema = tiptap_schema();
    let b = rendered_scalar_offset(&source, &schema, "B");
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![TypedOperation::InsertText {
            at: point_for_test(b),
            text: "X".into(),
            marks: vec![],
        }],
    );
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][0]["type"], "mysteryBlock");
    assert_eq!(actual["content"][1]["content"][0]["text"], "XB");
}

#[test]
fn malformed_wire_headings_remain_one_opaque_atom_and_hide_descendants() {
    for attrs in [
        None,
        Some(json!({ "level": 7 })),
        Some(json!({ "level": 2.5 })),
    ] {
        let mut heading = json!({
            "type": "heading",
            "content": [{ "type": "text", "text": "wire-only" }]
        });
        if let Some(attrs) = attrs {
            heading["attrs"] = attrs;
        }
        let source = json!({
            "type": "doc",
            "content": [
                heading,
                { "type": "paragraph", "content": [{ "type": "text", "text": "B" }] }
            ]
        });
        let schema = tiptap_schema();
        let b = rendered_scalar_offset(&source, &schema, "B");
        let (actual, expected, _, _, _) = compile_and_execute(
            source,
            vec![TypedOperation::InsertText {
                at: point_for_test(b),
                text: "X".into(),
                marks: vec![],
            }],
        );
        assert_eq!(actual, expected);
        assert_eq!(actual["content"][0]["type"], "heading");
        assert_eq!(actual["content"][1]["content"][0]["text"], "XB");
    }

    let source = json!({
        "type": "doc",
        "content": [
            {
                "type": "heading",
                "attrs": { "level": 7 },
                "content": [{ "type": "text", "text": "hidden" }]
            },
            { "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }
        ]
    });
    let (doc, schema, _, _, _) = diagnostic_doc(&source);
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let XmlOut::Element(heading) = fragment.get(&txn, 0).unwrap() else {
        panic!("heading wire element expected")
    };
    let XmlOut::Text(hidden) = heading.get(&txn, 0).unwrap() else {
        panic!("hidden wire text expected")
    };
    let descendant = StickyIndex::at(
        &txn,
        BranchPtr::from(<XmlTextRef as AsRef<Branch>>::as_ref(&hidden)),
        1,
        Assoc::After,
    )
    .unwrap();
    assert!(super::sticky_index_to_doc_pos(&txn, &fragment, &descendant, &schema).is_none());

    let valid_source = json!({
        "type": "doc",
        "content": [{
            "type": "h2",
            "content": [{ "type": "text", "text": "visible" }]
        }]
    });
    let (valid_doc, valid_schema, _, _, _) = diagnostic_doc(&valid_source);
    let valid_txn = valid_doc.transact();
    let valid_fragment = valid_txn.get_xml_fragment("prosemirror").unwrap();
    let XmlOut::Element(valid_heading) = valid_fragment.get(&valid_txn, 0).unwrap() else {
        panic!("valid heading wire element expected")
    };
    assert_eq!(valid_heading.tag().as_ref(), "heading");
    let XmlOut::Text(visible) = valid_heading.get(&valid_txn, 0).unwrap() else {
        panic!("valid heading text expected")
    };
    let visible_sticky = StickyIndex::at(
        &valid_txn,
        BranchPtr::from(<XmlTextRef as AsRef<Branch>>::as_ref(&visible)),
        1,
        Assoc::After,
    )
    .unwrap();
    assert_eq!(
        super::sticky_index_to_doc_pos(&valid_txn, &valid_fragment, &visible_sticky, &valid_schema,),
        Some(2)
    );
}

#[test]
fn shared_and_oversized_heading_levels_are_bounded_opaque_atoms() {
    let source = json!({
        "type": "doc",
        "content": [
            {
                "type": "h2",
                "content": [{ "type": "text", "text": "hidden" }]
            },
            { "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }
        ]
    });

    for shared_kind in 0..2 {
        let (doc, schema, limits, _, _) = diagnostic_doc(&source);
        let hidden = {
            let txn = doc.transact();
            let fragment = txn.get_xml_fragment("prosemirror").unwrap();
            let XmlOut::Element(heading) = fragment.get(&txn, 0).unwrap() else {
                panic!("heading expected")
            };
            let XmlOut::Text(text) = heading.get(&txn, 0).unwrap() else {
                panic!("heading text expected")
            };
            StickyIndex::at(
                &txn,
                BranchPtr::from(<XmlTextRef as AsRef<Branch>>::as_ref(&text)),
                1,
                Assoc::After,
            )
            .unwrap()
        };
        {
            let mut txn = doc.transact_mut();
            let fragment = txn.get_xml_fragment("prosemirror").unwrap();
            let XmlOut::Element(heading) = fragment.get(&txn, 0).unwrap() else {
                panic!("heading expected")
            };
            if shared_kind == 0 {
                heading.insert_attribute(
                    &mut txn,
                    "level",
                    MapPrelim::from([("nested", Any::String("2".into()))]),
                );
            } else {
                heading.insert_attribute(
                    &mut txn,
                    "level",
                    ArrayPrelim::from(vec![Any::String("2".into())]),
                );
            }
        }
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let XmlOut::Element(heading) = fragment.get(&txn, 0).unwrap() else {
            panic!("heading expected")
        };
        assert_eq!(
            super::codec::normalized_wire_element_node_type(&heading, &txn),
            "heading"
        );
        assert!(super::sticky_index_to_doc_pos(&txn, &fragment, &hidden, &schema).is_none());
        let after_atom = StickyIndex::at(
            &txn,
            BranchPtr::from(<yrs::types::xml::XmlFragmentRef as AsRef<Branch>>::as_ref(
                &fragment,
            )),
            1,
            Assoc::After,
        )
        .unwrap();
        assert_eq!(
            super::sticky_index_to_doc_pos(&txn, &fragment, &after_atom, &schema),
            Some(1)
        );
        let error = YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap_err();
        assert_eq!(error.code, "CODEC_INVARIANT_FAILED");
    }

    let (doc, schema, mut limits, _, _) = diagnostic_doc(&source);
    let oversized = format!("{}2", "0".repeat(128 * 1024));
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let XmlOut::Element(heading) = fragment.get(&txn, 0).unwrap() else {
            panic!("heading expected")
        };
        heading.insert_attribute(&mut txn, "level", oversized);
    }
    limits.max_input_bytes = 64;
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let XmlOut::Element(heading) = fragment.get(&txn, 0).unwrap() else {
        panic!("heading expected")
    };
    assert_eq!(
        super::codec::normalized_wire_element_node_type(&heading, &txn),
        "heading"
    );
    let error = YrsDocumentCodec::new(&schema, &limits)
        .read_json(&fragment, &txn)
        .unwrap_err();
    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(64));
}

#[test]
fn opaque_block_insert_inside_text_rejects_without_mutating_yrs() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "AB" }]
        }]
    });
    let (doc, schema, limits, editing_limits, document) = diagnostic_doc(&source);
    let opaque = Node::void(
        "__opaque_json".into(),
        HashMap::from([
            ("original_type".into(), Value::String("mysteryBlock".into())),
            (
                "original_json".into(),
                json!({ "type": "mysteryBlock", "content": [{ "type": "paragraph" }] }),
            ),
            ("opaque_placement".into(), Value::String("block".into())),
        ]),
    );
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let before = txn.encode_state_as_update_v1(&StateVector::default());
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
            request_id: 171,
            base_document_revision: 0,
            origin: TransactionOrigin::LocalCommand,
            operations: vec![TypedOperation::InsertNode {
                at: point_for_test(1),
                node: opaque,
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        },
        &txn,
        &fragment,
    )
    .unwrap_err();
    assert_eq!(error.code, "DOCUMENT_INVALID");
    assert_eq!(
        txn.encode_state_as_update_v1(&StateVector::default()),
        before
    );
    assert_eq!(
        YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap(),
        source
    );
}

#[test]
fn opaque_html_inline_and_block_insertions_round_trip_canonical_metadata() {
    let inline_attrs = HashMap::from([
        ("html_tag".into(), Value::String("widget-inline".into())),
        ("opaque_placement".into(), Value::String("inline".into())),
        ("html_attrs".into(), json!({ "data-id": "7" })),
        ("text_content".into(), Value::String("raw".into())),
        ("inner_html".into(), Value::String("<b>raw</b>".into())),
    ]);
    let inline = Node::void("__opaque".into(), inline_attrs);
    let inline_source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "ab" }]
        }]
    });
    let (inline_doc, inline_schema, inline_limits, inline_compiled) =
        compile_operations_with_schema(
            &inline_source,
            vec![
                TypedOperation::InsertNode {
                    at: point_for_test(1),
                    node: inline,
                },
                TypedOperation::InsertText {
                    at: point_for_test(1),
                    text: "X".into(),
                    marks: vec![],
                },
            ],
            tiptap_schema(),
        );
    let inline_expected = to_prosemirror_json(&inline_compiled.preview, &inline_schema);
    assert_eq!(
        to_html(&inline_compiled.preview, &inline_schema),
        "<p>a<widget-inline data-id=\"7\"><b>raw</b></widget-inline>Xb</p>"
    );
    {
        let mut txn = inline_doc.transact_mut();
        execute_mutation_plan(inline_compiled.mutation_plan, &mut txn);
    }
    let txn = inline_doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let actual = YrsDocumentCodec::new(&inline_schema, &inline_limits)
        .read_json(&fragment, &txn)
        .unwrap();
    assert_eq!(actual, inline_expected);
    assert_eq!(actual["content"][0]["content"][1]["type"], "__opaque");
    assert_eq!(actual["content"][0]["content"][2]["text"], "Xb");

    let block_attrs = HashMap::from([
        ("html_tag".into(), Value::String("widget-block".into())),
        ("opaque_placement".into(), Value::String("block".into())),
        ("html_attrs".into(), json!({ "data-kind": "card" })),
        ("inner_html".into(), Value::String("<i>card</i>".into())),
    ]);
    let block = Node::void("__opaque".into(), block_attrs);
    let block_source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "A" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "B" }] }
        ]
    });
    let schema = tiptap_schema();
    let at = rendered_scalar_offset(&block_source, &schema, "B") - 1;
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &block_source,
        vec![TypedOperation::InsertNode {
            at: point_for_test(at),
            node: block,
        }],
        schema,
    );
    assert_eq!(
        to_html(&compiled.preview, &schema),
        "<p>A</p><widget-block data-kind=\"card\"><i>card</i></widget-block><p>B</p>"
    );
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    assert_eq!(
        YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap(),
        expected
    );
}

#[test]
fn opaque_sentinel_validator_rejects_forged_shapes_and_known_aliases() {
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let json_attrs = |original_type: &str, original_json: Value| {
        HashMap::from([
            ("original_type".into(), Value::String(original_type.into())),
            ("original_json".into(), original_json),
            ("opaque_placement".into(), Value::String("inline".into())),
        ])
    };
    let mut forged = vec![
        Node::element(
            "__opaque_json".into(),
            json_attrs("mystery", json!({ "type": "mystery" })),
            Fragment::from(vec![Node::text("child".into(), vec![])]),
        ),
        Node::void(
            "__opaque_json".into(),
            json_attrs("mystery", json!({ "type": "different" })),
        ),
        Node::void(
            "__opaque_json".into(),
            HashMap::from([
                ("original_type".into(), Value::String("mystery".into())),
                ("opaque_placement".into(), Value::String("inline".into())),
            ]),
        ),
        Node::void(
            "__opaque_json".into(),
            json_attrs("mystery", Value::String("not-an-object".into())),
        ),
        Node::void(
            "__opaque_json".into(),
            json_attrs("__opaque", json!({ "type": "__opaque" })),
        ),
        Node::void("__opaque_json".into(), {
            let mut attrs = json_attrs("mystery", json!({ "type": "mystery" }));
            attrs.insert("extra".into(), Value::Bool(true));
            attrs
        }),
        Node::void(
            "__opaque_json".into(),
            json_attrs("paragraph", json!({ "type": "paragraph" })),
        ),
        Node::void(
            "__opaque_json".into(),
            json_attrs(
                "heading",
                json!({ "type": "heading", "attrs": { "level": 2 } }),
            ),
        ),
        Node::void(
            "__opaque_json".into(),
            json_attrs(
                "heading",
                json!({ "type": "heading", "attrs": { "level": "2" } }),
            ),
        ),
        Node::void(
            "__opaque".into(),
            HashMap::from([
                ("html_tag".into(), Value::String("Bad<Tag".into())),
                ("opaque_placement".into(), Value::String("inline".into())),
            ]),
        ),
        Node::element(
            "__opaque".into(),
            HashMap::from([
                ("html_tag".into(), Value::String("widget-inline".into())),
                ("opaque_placement".into(), Value::String("inline".into())),
            ]),
            Fragment::from(vec![Node::text("child".into(), vec![])]),
        ),
        Node::void(
            "__opaque".into(),
            HashMap::from([
                ("html_tag".into(), Value::String("strong".into())),
                ("opaque_placement".into(), Value::String("inline".into())),
            ]),
        ),
        Node::void(
            "__opaque".into(),
            HashMap::from([
                ("html_tag".into(), Value::String("widget-inline".into())),
                ("opaque_placement".into(), Value::String("inline".into())),
                ("html_attrs".into(), json!({ "bad key": "value" })),
            ]),
        ),
        Node::void(
            "__opaque".into(),
            HashMap::from([
                ("html_tag".into(), Value::String("widget-inline".into())),
                ("opaque_placement".into(), Value::String("inline".into())),
                ("html_attrs".into(), json!({ "data-id": 7 })),
            ]),
        ),
        Node::void(
            "__opaque".into(),
            HashMap::from([
                ("html_tag".into(), Value::String("widget-inline".into())),
                ("opaque_placement".into(), Value::String("inline".into())),
                ("extra".into(), Value::Bool(true)),
            ]),
        ),
        Node::void(
            "__opaque".into(),
            HashMap::from([
                ("html_tag".into(), Value::String("img".into())),
                ("opaque_placement".into(), Value::String("block".into())),
                (
                    "html_attrs".into(),
                    json!({ "src": "https://example.test/image.png" }),
                ),
            ]),
        ),
    ];
    for tag in ["b", "i", "del", "strike"] {
        forged.push(Node::void(
            "__opaque".into(),
            HashMap::from([
                ("html_tag".into(), Value::String(tag.into())),
                ("opaque_placement".into(), Value::String("inline".into())),
            ]),
        ));
    }
    forged.push(Node::void(
        "__opaque".into(),
        HashMap::from([
            ("html_tag".into(), Value::String("span".into())),
            ("opaque_placement".into(), Value::String("inline".into())),
            (
                "html_attrs".into(),
                json!({ "data-native-editor-mark": "bold" }),
            ),
        ]),
    ));
    for (case_index, opaque) in forged.into_iter().enumerate() {
        let document = Document::new(Node::element(
            "doc".into(),
            HashMap::new(),
            Fragment::from(vec![Node::element(
                "paragraph".into(),
                HashMap::new(),
                Fragment::from(vec![opaque]),
            )]),
        ));
        let error = match DocumentValidator::validate(&document, &schema, &limits) {
            Ok(_) => panic!("forged opaque case {case_index} was accepted"),
            Err(error) => error,
        };
        assert_eq!(error.code, "DOCUMENT_INVALID");
    }

    for (tag, html_attrs, placement) in [
        ("paragraph", json!({}), "inline"),
        (
            "img",
            json!({ "src": "https://example.test/image.png", "alt": "Inline" }),
            "inline",
        ),
        (
            "img",
            json!({ "src": "data:image/png;base64,AAAA", "alt": "Inline" }),
            "block",
        ),
    ] {
        let opaque = Node::void(
            "__opaque".into(),
            HashMap::from([
                ("html_tag".into(), Value::String(tag.into())),
                ("opaque_placement".into(), Value::String(placement.into())),
                ("html_attrs".into(), html_attrs),
            ]),
        );
        let document = Document::new(Node::element(
            "doc".into(),
            HashMap::new(),
            Fragment::from(if placement == "block" {
                vec![opaque]
            } else {
                vec![Node::element(
                    "paragraph".into(),
                    HashMap::new(),
                    Fragment::from(vec![opaque]),
                )]
            }),
        ));
        DocumentValidator::validate(&document, &schema, &limits).unwrap();
    }

    let semantic_block_image = Document::new(Node::element(
        "doc".into(),
        HashMap::new(),
        Fragment::from(vec![Node::void(
            "__opaque".into(),
            HashMap::from([
                ("html_tag".into(), Value::String("img".into())),
                ("opaque_placement".into(), Value::String("block".into())),
                (
                    "html_attrs".into(),
                    json!({ "src": "https://example.test/image.png" }),
                ),
            ]),
        )]),
    ));
    assert_eq!(
        DocumentValidator::validate(&semantic_block_image, &schema, &limits)
            .unwrap_err()
            .code,
        "DOCUMENT_INVALID"
    );

    let mention_schema = Schema::from_json(&json!({
        "nodes": [
            { "name": "doc", "content": "block*", "role": "doc" },
            { "name": "paragraph", "content": "inline*", "group": "block", "role": "textBlock", "htmlTag": "p" },
            { "name": "mention", "content": "", "group": "inline", "role": "inline", "isVoid": true },
            { "name": "text", "group": "inline", "role": "text" }
        ],
        "marks": []
    }))
    .unwrap();
    let mention_forge = Document::new(Node::element(
        "doc".into(),
        HashMap::new(),
        Fragment::from(vec![Node::element(
            "paragraph".into(),
            HashMap::new(),
            Fragment::from(vec![Node::void(
                "__opaque".into(),
                HashMap::from([
                    ("html_tag".into(), Value::String("span".into())),
                    ("opaque_placement".into(), Value::String("inline".into())),
                    (
                        "html_attrs".into(),
                        json!({ "data-native-editor-mention": "true" }),
                    ),
                ]),
            )]),
        )]),
    ));
    assert_eq!(
        DocumentValidator::validate(&mention_forge, &mention_schema, &limits)
            .unwrap_err()
            .code,
        "DOCUMENT_INVALID"
    );

    let inline_void_schema = Schema::from_json(&json!({
        "nodes": [
            { "name": "doc", "content": "block+", "role": "doc" },
            { "name": "paragraph", "content": "inline*", "group": "block", "role": "textBlock", "htmlTag": "p" },
            { "name": "inlineVoid", "content": "", "group": "inline", "role": "inline", "htmlTag": "x-void", "isVoid": true },
            { "name": "hardBreak", "content": "", "group": "inline", "role": "hardBreak", "htmlTag": "br", "isVoid": true },
            { "name": "text", "group": "inline", "role": "text" }
        ],
        "marks": []
    }))
    .unwrap();
    for tag in ["x-void", "br"] {
        let forged = Document::new(Node::element(
            "doc".into(),
            HashMap::new(),
            Fragment::from(vec![Node::element(
                "paragraph".into(),
                HashMap::new(),
                Fragment::from(vec![Node::void(
                    "__opaque".into(),
                    HashMap::from([
                        ("html_tag".into(), Value::String(tag.into())),
                        ("opaque_placement".into(), Value::String("inline".into())),
                    ]),
                )]),
            )]),
        ));
        assert_eq!(
            DocumentValidator::validate(&forged, &inline_void_schema, &limits)
                .unwrap_err()
                .code,
            "DOCUMENT_INVALID"
        );
    }
}

#[test]
fn uppercase_private_opaque_html_attributes_are_rejected_before_reimport_normalizes_them() {
    let limits = ResourceLimits::default();
    let mark_schema = tiptap_schema();
    let mark_forge = Document::new(Node::element(
        "doc".into(),
        HashMap::new(),
        Fragment::from(vec![Node::element(
            "paragraph".into(),
            HashMap::new(),
            Fragment::from(vec![Node::void(
                "__opaque".into(),
                HashMap::from([
                    ("html_tag".into(), Value::String("span".into())),
                    ("opaque_placement".into(), Value::String("inline".into())),
                    (
                        "html_attrs".into(),
                        json!({ "DATA-NATIVE-EDITOR-MARK": "bold" }),
                    ),
                    ("inner_html".into(), Value::String("marked".into())),
                ]),
            )]),
        )]),
    ));
    assert_eq!(
        DocumentValidator::validate(&mark_forge, &mark_schema, &limits)
            .unwrap_err()
            .code,
        "DOCUMENT_INVALID"
    );
    let mark_html = to_html(&mark_forge, &mark_schema);
    let reparsed_mark = from_html(&mark_html, &mark_schema, &FromHtmlOptions::default()).unwrap();
    let reparsed_mark_json = to_prosemirror_json(&reparsed_mark, &mark_schema);
    assert_eq!(
        reparsed_mark_json["content"][0]["content"][0]["marks"][0]["type"],
        "bold"
    );

    let mention_schema = Schema::from_json(&json!({
        "nodes": [
            { "name": "doc", "content": "block*", "role": "doc" },
            { "name": "paragraph", "content": "inline*", "group": "block", "role": "textBlock", "htmlTag": "p" },
            { "name": "mention", "content": "", "group": "inline", "role": "inline", "isVoid": true },
            { "name": "text", "group": "inline", "role": "text" }
        ],
        "marks": []
    }))
    .unwrap();
    let mention_forge = Document::new(Node::element(
        "doc".into(),
        HashMap::new(),
        Fragment::from(vec![Node::element(
            "paragraph".into(),
            HashMap::new(),
            Fragment::from(vec![Node::void(
                "__opaque".into(),
                HashMap::from([
                    ("html_tag".into(), Value::String("span".into())),
                    ("opaque_placement".into(), Value::String("inline".into())),
                    (
                        "html_attrs".into(),
                        json!({ "DATA-NATIVE-EDITOR-MENTION": "true" }),
                    ),
                    ("inner_html".into(), Value::String("@Ada".into())),
                ]),
            )]),
        )]),
    ));
    assert_eq!(
        DocumentValidator::validate(&mention_forge, &mention_schema, &limits)
            .unwrap_err()
            .code,
        "DOCUMENT_INVALID"
    );
    let mention_html = to_html(&mention_forge, &mention_schema);
    let reparsed_mention =
        from_html(&mention_html, &mention_schema, &FromHtmlOptions::default()).unwrap();
    assert_eq!(
        to_prosemirror_json(&reparsed_mention, &mention_schema)["content"][0]["content"][0]["type"],
        "mention"
    );
}

#[test]
fn non_span_private_mention_metadata_remains_opaque_after_export_and_reimport() {
    let schema = Schema::from_json(&json!({
        "nodes": [
            { "name": "doc", "content": "block*", "role": "doc" },
            { "name": "paragraph", "content": "inline*", "group": "block", "role": "textBlock", "htmlTag": "p" },
            { "name": "mention", "content": "", "group": "inline", "role": "inline", "isVoid": true },
            { "name": "text", "group": "inline", "role": "text" }
        ],
        "marks": []
    }))
    .unwrap();
    let document = Document::new(Node::element(
        "doc".into(),
        HashMap::new(),
        Fragment::from(vec![Node::element(
            "paragraph".into(),
            HashMap::new(),
            Fragment::from(vec![Node::void(
                "__opaque".into(),
                HashMap::from([
                    ("html_tag".into(), Value::String("x-mention".into())),
                    ("opaque_placement".into(), Value::String("inline".into())),
                    (
                        "html_attrs".into(),
                        json!({ "data-native-editor-mention": "true" }),
                    ),
                    ("inner_html".into(), Value::String("@Ada".into())),
                ]),
            )]),
        )]),
    ));
    DocumentValidator::validate(&document, &schema, &ResourceLimits::default()).unwrap();
    let html = to_html(&document, &schema);
    let reparsed = from_html(&html, &schema, &FromHtmlOptions::default()).unwrap();
    let json = to_prosemirror_json(&reparsed, &schema);
    assert_eq!(json["content"][0]["content"][0]["type"], "__opaque");
    assert_eq!(
        json["content"][0]["content"][0]["attrs"]["html_attrs"]["data-native-editor-mention"],
        "true"
    );
}

#[test]
fn canonical_foreign_mixed_case_attributes_validate_and_round_trip() {
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    for (tag, key, value) in [
        ("svg", "viewBox", "0 0 10 10"),
        ("math", "definitionURL", "https://example.test/definition"),
    ] {
        let document = Document::new(Node::element(
            "doc".into(),
            HashMap::new(),
            Fragment::from(vec![Node::element(
                "paragraph".into(),
                HashMap::new(),
                Fragment::from(vec![Node::void(
                    "__opaque".into(),
                    HashMap::from([
                        ("html_tag".into(), Value::String(tag.into())),
                        ("opaque_placement".into(), Value::String("inline".into())),
                        ("html_attrs".into(), json!({ key: value })),
                    ]),
                )]),
            )]),
        ));
        DocumentValidator::validate(&document, &schema, &limits).unwrap();
        let html = to_html(&document, &schema);
        let reparsed = from_html(&html, &schema, &FromHtmlOptions::default()).unwrap();
        let json = to_prosemirror_json(&reparsed, &schema);
        assert_eq!(
            json["content"][0]["content"][0]["attrs"]["html_attrs"][key],
            value
        );
    }

    for (tag, key) in [
        ("a", "attributeName"),
        ("svg", "DATA-NATIVE-EDITOR-MENTION"),
    ] {
        let forged = Document::new(Node::element(
            "doc".into(),
            HashMap::new(),
            Fragment::from(vec![Node::element(
                "paragraph".into(),
                HashMap::new(),
                Fragment::from(vec![Node::void(
                    "__opaque".into(),
                    HashMap::from([
                        ("html_tag".into(), Value::String(tag.into())),
                        ("opaque_placement".into(), Value::String("inline".into())),
                        ("html_attrs".into(), json!({ key: "forged" })),
                    ]),
                )]),
            )]),
        ));
        assert_eq!(
            DocumentValidator::validate(&forged, &schema, &limits)
                .unwrap_err()
                .code,
            "DOCUMENT_INVALID"
        );
    }
    for (tag, html_attrs) in [
        (
            "svg",
            json!({ "viewBox": "0 0 10 10", "viewbox": "0 0 20 20" }),
        ),
        (
            "math",
            json!({ "definitionURL": "a", "definitionurl": "b" }),
        ),
    ] {
        let collision = Document::new(Node::element(
            "doc".into(),
            HashMap::new(),
            Fragment::from(vec![Node::element(
                "paragraph".into(),
                HashMap::new(),
                Fragment::from(vec![Node::void(
                    "__opaque".into(),
                    HashMap::from([
                        ("html_tag".into(), Value::String(tag.into())),
                        ("opaque_placement".into(), Value::String("inline".into())),
                        ("html_attrs".into(), html_attrs),
                    ]),
                )]),
            )]),
        ));
        assert_eq!(
            DocumentValidator::validate(&collision, &schema, &limits)
                .unwrap_err()
                .code,
            "DOCUMENT_INVALID"
        );
    }
}

#[test]
fn foreign_qualified_attributes_preserve_prefixes_without_colliding() {
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let document = Document::new(Node::element(
        "doc".into(),
        HashMap::new(),
        Fragment::from(vec![Node::element(
            "paragraph".into(),
            HashMap::new(),
            Fragment::from(vec![Node::void(
                "__opaque".into(),
                HashMap::from([
                    ("html_tag".into(), Value::String("svg".into())),
                    ("opaque_placement".into(), Value::String("inline".into())),
                    (
                        "html_attrs".into(),
                        json!({
                            "href": "plain",
                            "xlink:href": "linked",
                            "xml:lang": "en",
                            "xmlns:xlink": "http://www.w3.org/1999/xlink"
                        }),
                    ),
                ]),
            )]),
        )]),
    ));
    DocumentValidator::validate(&document, &schema, &limits).unwrap();
    let expected = to_prosemirror_json(&document, &schema);
    let html = to_html(&document, &schema);
    let reparsed = from_html(&html, &schema, &FromHtmlOptions::default()).unwrap();
    let actual = to_prosemirror_json(&reparsed, &schema);
    assert_eq!(actual, expected);
}

#[test]
fn malformed_reserved_opaque_insert_rejects_atomically_before_yrs_execution() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "AB" }]
        }]
    });
    let (doc, schema, limits, editing_limits, document) = diagnostic_doc(&source);
    let malformed = Node::void(
        "__opaque_json".into(),
        HashMap::from([
            ("original_type".into(), Value::String("mystery".into())),
            ("original_json".into(), Value::Null),
            ("opaque_placement".into(), Value::String("inline".into())),
        ]),
    );
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let before = txn.encode_state_as_update_v1(&StateVector::default());
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
            request_id: 172,
            base_document_revision: 0,
            origin: TransactionOrigin::LocalCommand,
            operations: vec![TypedOperation::InsertNode {
                at: point_for_test(1),
                node: malformed,
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        },
        &txn,
        &fragment,
    )
    .unwrap_err();
    assert_eq!(error.code, "DOCUMENT_INVALID");
    assert_eq!(
        txn.encode_state_as_update_v1(&StateVector::default()),
        before
    );
    assert_eq!(
        YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap(),
        source
    );
}

#[test]
fn opaque_metadata_depth_and_width_limits_are_exact_and_atomic() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "AB" }]
        }]
    });
    let (doc, schema, limits, editing_limits, document) = diagnostic_doc(&source);
    let compile = |resource_limits: &ResourceLimits, node: Node| {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        compile_transaction_with_yrs(
            CompilationContext {
                document: &document,
                selection: None,
                schema: &schema,
                resource_limits,
                editing_limits: &editing_limits,
                document_revision: 0,
                max_length: None,
            },
            TypedTransaction {
                request_id: 173,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalCommand,
                operations: vec![TypedOperation::InsertNode {
                    at: point_for_test(1),
                    node,
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
    };
    let nested = || {
        Node::void(
            "__opaque_json".into(),
            HashMap::from([
                ("original_type".into(), Value::String("mystery".into())),
                (
                    "original_json".into(),
                    json!({
                        "type": "mystery",
                        "attrs": { "payload": [[[0]]] }
                    }),
                ),
                ("opaque_placement".into(), Value::String("inline".into())),
            ]),
        )
    };
    let mut exact_depth = limits.clone();
    exact_depth.max_document_depth = 6;
    compile(&exact_depth, nested()).unwrap();
    let txn = doc.transact();
    let before = txn.encode_state_as_update_v1(&StateVector::default());
    drop(txn);
    exact_depth.max_document_depth = 5;
    let error = compile(&exact_depth, nested()).unwrap_err();
    assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(5));
    assert_eq!(error.actual, Some(6));

    let html_attrs = (0..100)
        .map(|index| (format!("data-{index}"), Value::String("x".into())))
        .collect::<serde_json::Map<_, _>>();
    let wide = || {
        Node::void(
            "__opaque".into(),
            HashMap::from([
                ("html_tag".into(), Value::String("widget-inline".into())),
                ("opaque_placement".into(), Value::String("inline".into())),
                ("html_attrs".into(), Value::Object(html_attrs.clone())),
            ]),
        )
    };
    let mut exact_width = limits.clone();
    exact_width.max_document_nodes = 103;
    compile(&exact_width, wide()).unwrap();
    exact_width.max_document_nodes = 102;
    let error = compile(&exact_width, wide()).unwrap_err();
    assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(102));
    assert_eq!(error.actual, Some(103));
    let txn = doc.transact();
    assert_eq!(
        txn.encode_state_as_update_v1(&StateVector::default()),
        before
    );
}

#[test]
fn opaque_metadata_max_input_bytes_is_exact_aggregated_and_atomic() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "AB" }]
        }]
    });
    let (doc, schema, limits, editing_limits, document) = diagnostic_doc(&source);
    let escaped_payload = "\u{0001}".repeat(4_096);
    let make_node = || {
        Node::void(
            "__opaque_json".into(),
            HashMap::from([
                ("original_type".into(), Value::String("mystery".into())),
                (
                    "original_json".into(),
                    json!({ "type": "mystery", "attrs": { "payload": escaped_payload } }),
                ),
                ("opaque_placement".into(), Value::String("inline".into())),
            ]),
        )
    };
    let exact_input = {
        let node = make_node();
        node.node_type().len() + serde_json::to_vec(node.attrs()).unwrap().len()
    };
    let compile = |resource_limits: &ResourceLimits| {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        compile_transaction_with_yrs(
            CompilationContext {
                document: &document,
                selection: None,
                schema: &schema,
                resource_limits,
                editing_limits: &editing_limits,
                document_revision: 0,
                max_length: None,
            },
            TypedTransaction {
                request_id: 174,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalCommand,
                operations: vec![TypedOperation::InsertNode {
                    at: point_for_test(1),
                    node: make_node(),
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
    };
    let initial_scan = {
        let txn = doc.transact();
        document.root().text_content().len() * 2
            + crdt_clock_scan_reservation(174, &txn, limits.max_encoded_state_bytes).unwrap() * 2
    };
    let baseline = compile(&limits).unwrap();
    let envelope_scan = {
        let txn = doc.transact();
        crdt_envelope(174, &txn, limits.max_encoded_state_bytes)
            .unwrap()
            .scan_work
    };
    let exact_total =
        (exact_input + initial_scan).max(baseline.mutation_plan.scan_work + envelope_scan);
    let mut exact_limits = limits.clone();
    exact_limits.max_input_bytes = exact_total;
    let admitted = compile(&exact_limits).unwrap();
    assert!(admitted.mutation_plan.scan_work < exact_total);

    let txn = doc.transact();
    let before = txn.encode_state_as_update_v1(&StateVector::default());
    drop(txn);
    exact_limits.max_input_bytes = exact_total - 1;
    let error = compile(&exact_limits).unwrap_err();
    assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(u64::try_from(exact_total - 1).unwrap()));
    assert_eq!(error.actual, Some(u64::try_from(exact_total).unwrap()));
    assert_eq!(error.details, Some(json!({ "field": "maxInputBytes" })));
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    assert_eq!(
        txn.encode_state_as_update_v1(&StateVector::default()),
        before
    );
    assert_eq!(
        YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap(),
        source
    );
}

#[test]
fn sticky_reverse_mapping_rejects_unknown_wire_element_and_descendant_branches() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{
                "type": "mysteryInline",
                "content": [{ "type": "text", "text": "hidden" }]
            }]
        }]
    });
    let (doc, schema, _, _, _) = diagnostic_doc(&source);
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let XmlOut::Element(paragraph) = fragment.get(&txn, 0).unwrap() else {
        panic!("paragraph expected")
    };
    let XmlOut::Element(unknown) = paragraph.get(&txn, 0).unwrap() else {
        panic!("unknown element expected")
    };
    let XmlOut::Text(hidden) = unknown.get(&txn, 0).unwrap() else {
        panic!("hidden text expected")
    };
    let paragraph_branch = BranchPtr::from(
        <yrs::types::xml::XmlElementRef as AsRef<Branch>>::as_ref(&paragraph),
    );
    for (position, sticky) in [
        (
            1,
            StickyIndex::at(&txn, paragraph_branch, 0, Assoc::Before).unwrap(),
        ),
        (
            2,
            StickyIndex::at(&txn, paragraph_branch, 1, Assoc::Before).unwrap(),
        ),
    ] {
        assert_eq!(
            super::sticky_index_to_doc_pos(&txn, &fragment, &sticky, &schema),
            Some(position)
        );
    }
    for position in [1, 2] {
        for affinity in [Affinity::Before, Affinity::After] {
            let point =
                super::doc_pos_to_relative_point(&txn, &fragment, position, affinity, &schema)
                    .unwrap();
            assert_eq!(point.affinity, affinity);
            assert_eq!(
                super::relative_point_to_doc_pos(&txn, &fragment, &point, &schema),
                Some(position)
            );
        }
    }
    for sticky in [
        StickyIndex::at(
            &txn,
            BranchPtr::from(<yrs::types::xml::XmlElementRef as AsRef<Branch>>::as_ref(
                &unknown,
            )),
            0,
            Assoc::After,
        )
        .unwrap(),
        StickyIndex::at(
            &txn,
            BranchPtr::from(<XmlTextRef as AsRef<Branch>>::as_ref(&hidden)),
            1,
            Assoc::After,
        )
        .unwrap(),
    ] {
        assert!(super::sticky_index_to_doc_pos(&txn, &fragment, &sticky, &schema).is_none());
    }
}

#[test]
fn structural_insert_splits_one_marked_unicode_storage_text_exactly() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "A😀e\u{301}Z" }]
        }]
    });
    let (doc, schema, limits, editing_limits, _) = diagnostic_doc(&source);
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let text = paragraph_text(&fragment, &txn, 0);
        text.format(
            &mut txn,
            3,
            2,
            Attrs::from([(Arc::<str>::from("bold"), Any::Bool(true))]),
        );
    }
    let codec = YrsDocumentCodec::new(&schema, &limits);
    let (document, original_id, before_full_len) = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let json = codec.read_json(&fragment, &txn).unwrap();
        (
            from_prosemirror_json(&json, &schema, UnknownTypeMode::Preserve).unwrap(),
            <XmlTextRef as AsRef<Branch>>::as_ref(&paragraph_text(&fragment, &txn, 0)).id(),
            txn.encode_state_as_update_v1(&StateVector::default()).len(),
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
                request_id: 113,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertNode {
                    at: point_for_test(2),
                    node: Node::void("hardBreak".into(), HashMap::new()),
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
        Some(YrsMutationAction::DeleteText {
            index_utf16: 3,
            len_utf16: 3,
            ..
        })
    ));
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    let estimate = compiled.encoded_growth_bound;
    let update = {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
        txn.commit();
        txn.encode_update_v1()
    };
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let XmlOut::Element(paragraph) = fragment.get(&txn, 0).unwrap() else {
        panic!("expected paragraph")
    };
    let children = paragraph.children(&txn).collect::<Vec<_>>();
    assert_eq!(children[0].id(), original_id);
    assert_ne!(children[2].id(), original_id);
    assert_eq!(codec.read_json(&fragment, &txn).unwrap(), expected);
    assert_eq!(expected["content"][0]["content"][0]["text"], "A😀");
    assert_eq!(expected["content"][0]["content"][2]["text"], "e\u{301}");
    assert_eq!(
        expected["content"][0]["content"][2]["marks"][0]["type"],
        "bold"
    );
    let update_len = update.len();
    assert!(update_len <= estimate, "{update_len} > {estimate}");
    let after_full_len = txn.encode_state_as_update_v1(&StateVector::default()).len();
    assert!(after_full_len <= before_full_len + estimate);
}

#[test]
fn structural_replace_swaps_an_inline_void_for_text_at_the_same_index() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "hardBreak" }]
        }]
    });

    let (actual, expected, html, update_len, estimate) = compile_and_execute(
        source,
        vec![TypedOperation::ReplaceRange {
            range: range_for_test(0, 1),
            content: Fragment::from(vec![Node::text("x".into(), vec![])]),
        }],
    );

    assert_eq!(actual, expected);
    assert_eq!(actual["content"][0]["content"][0]["text"], "x");
    assert!(html.contains(">x<"));
    assert!(update_len > 0);
    assert!(update_len <= estimate);
}

#[test]
fn structurally_identical_replace_is_a_document_no_op() {
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
                request_id: 116,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::ReplaceRange {
                    range: range_for_test(0, 1),
                    content: Fragment::from(vec![Node::void("hardBreak".into(), HashMap::new())]),
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
        .unwrap()
    };
    assert!(compiled.mutation_plan.actions.is_empty());
    assert_eq!(compiled.encoded_growth_bound, 0);
    assert_eq!(compiled.undo_units_bound, 0);
}

#[test]
fn update_node_attrs_toggles_and_removes_task_item_default() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "taskList",
            "content": [{
                "type": "taskItem",
                "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "todo" }] }]
            }]
        }]
    });
    let (actual, expected) = compile_and_execute_attribute_update(
        source,
        HashMap::from([("checked".into(), Value::Bool(true))]),
    );
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][0]["content"][0]["attrs"]["checked"], true);

    let checked_source = json!({
        "type": "doc",
        "content": [{
            "type": "taskList",
            "content": [{
                "type": "taskItem",
                "attrs": { "checked": true },
                "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "done" }] }]
            }]
        }]
    });
    let (removed, removed_expected) =
        compile_and_execute_attribute_update(checked_source, HashMap::new());
    assert_eq!(removed, removed_expected);
    assert!(removed["content"][0]["content"][0]["attrs"]["checked"].is_null());
}

#[test]
fn update_node_attrs_sets_and_removes_image_attributes() {
    let source = json!({
        "type": "doc",
        "content": [{ "type": "image", "attrs": { "src": "old", "alt": "old alt" } }]
    });
    let (actual, expected) = compile_and_execute_attribute_update(
        source,
        HashMap::from([("src".into(), Value::String("new".into()))]),
    );
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][0]["attrs"]["src"], "new");
    assert!(actual["content"][0]["attrs"]["alt"].is_null());
}

#[test]
fn update_node_attrs_preserves_nested_custom_any_values() {
    let source = json!({
        "type": "doc",
        "content": [{ "type": "customBlock", "attrs": { "old": true } }]
    });
    let attrs = HashMap::from([
        ("flag".into(), Value::Bool(true)),
        ("count".into(), json!(7)),
        ("label".into(), Value::String("custom".into())),
        ("items".into(), json!([1, false, "x"])),
        ("meta".into(), json!({ "nested": { "ok": true } })),
    ]);
    let (actual, expected) = compile_and_execute_attribute_update(source, attrs);
    assert_eq!(actual, expected);
    assert_eq!(
        actual["content"][0]["attrs"]["items"],
        json!([1, false, "x"])
    );
    assert_eq!(
        actual["content"][0]["attrs"]["meta"],
        json!({ "nested": { "ok": true } })
    );
}

#[test]
fn update_node_attrs_normalizes_sequential_same_key_changes() {
    let source = json!({
        "type": "doc",
        "content": [{ "type": "customBlock", "attrs": { "old": true } }]
    });
    let (_, _, _, compiled) = compile_attribute_operations(
        source.clone(),
        vec![
            HashMap::from([("label".into(), Value::String("first".into()))]),
            HashMap::from([("label".into(), Value::String("final".into()))]),
        ],
    );
    assert_eq!(compiled.mutation_plan.actions.len(), 2);
    assert!(matches!(
        compiled.mutation_plan.actions.as_slice(),
        [
            YrsMutationAction::SetXmlAttribute { key, value: Any::String(value), .. },
            YrsMutationAction::RemoveXmlAttribute { key: removed, .. }
        ] if key.as_ref() == "label" && value.as_ref() == "final" && removed.as_ref() == "old"
    ));

    let (_, _, _, removed) = compile_attribute_operations(
        source,
        vec![
            HashMap::from([("label".into(), Value::String("temporary".into()))]),
            HashMap::new(),
        ],
    );
    assert!(matches!(
        removed.mutation_plan.actions.as_slice(),
        [YrsMutationAction::RemoveXmlAttribute { key, .. }] if key.as_ref() == "old"
    ));
}

#[test]
fn update_node_attrs_identical_map_is_a_complete_no_op() {
    let source = json!({
        "type": "doc",
        "content": [{ "type": "customBlock", "attrs": { "flag": true } }]
    });
    let (_, _, _, compiled) = compile_attribute_operations(
        source,
        vec![HashMap::from([("flag".into(), Value::Bool(true))])],
    );
    assert!(compiled.mutation_plan.actions.is_empty());
    assert_eq!(compiled.encoded_growth_bound, 0);
    assert_eq!(compiled.undo_units_bound, 0);
}

#[test]
fn update_node_attrs_rejects_stale_same_count_attribute_substitution_atomically() {
    let source = json!({
        "type": "doc",
        "content": [{ "type": "image", "attrs": { "src": "old", "alt": "old alt" } }]
    });
    let (doc, _, _, compiled) = compile_attribute_operations(
        source,
        vec![HashMap::from([("src".into(), Value::String("new".into()))])],
    );
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let XmlOut::Element(image) = fragment.get(&txn, 0).unwrap() else {
            panic!("expected image")
        };
        image.insert_attribute(&mut txn, "src", Any::String("raced".into()));
    }
    let before = doc
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    let error = {
        let txn = doc.transact();
        preflight_mutation_plan(118, &compiled.mutation_plan, &txn).unwrap_err()
    };
    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(
        doc.transact()
            .encode_state_as_update_v1(&StateVector::default()),
        before
    );
}

#[test]
fn update_node_attrs_keeps_heading_synthetic_level_unchanged() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "h2",
            "attrs": { "id": "old" },
            "content": [{ "type": "text", "text": "Heading" }]
        }]
    });
    let (_, _, _, compiled) = compile_attribute_operations(
        source,
        vec![HashMap::from([("id".into(), Value::String("new".into()))])],
    );
    assert!(matches!(
        compiled.mutation_plan.actions.as_slice(),
        [YrsMutationAction::SetXmlAttribute { key, .. }] if key.as_ref() == "id"
    ));
}

#[test]
fn update_node_attrs_rejects_ambiguous_attrless_target() {
    let source = json!({
        "type": "doc",
        "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "x" }] }]
    });
    let (doc, schema, limits, editing_limits, document) =
        diagnostic_doc_with_schema(&source, attribute_schema());
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
            request_id: 119,
            base_document_revision: 0,
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::UpdateNodeAttrs {
                at: point_for_test(0),
                attrs: HashMap::new(),
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        },
        &txn,
        &fragment,
    )
    .unwrap_err();
    assert_eq!(error.code, "POSITION_INVALID");
    assert_eq!(error.details.as_ref().unwrap()["field"], "at");
}

#[test]
fn split_block_directly_preserves_left_marked_unicode_storage() {
    let source = json!({
        "type": "doc",
        "content": [
            {
                "type": "paragraph",
                "content": [{ "type": "text", "text": "A😀B", "marks": [{ "type": "bold" }] }]
            },
            { "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }
        ]
    });
    let (doc, schema, limits, editing_limits, document) = diagnostic_doc(&source);
    let codec = YrsDocumentCodec::new(&schema, &limits);
    let (left_block_id, left_text_id, tail_block_id, tail_text_id, tail_sticky, before_full_len) = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let children = fragment.children(&txn).collect::<Vec<_>>();
        let left_text = paragraph_text(&fragment, &txn, 0);
        let tail_text = paragraph_text(&fragment, &txn, 1);
        let tail_sticky = StickyIndex::at(
            &txn,
            BranchPtr::from(<XmlTextRef as AsRef<Branch>>::as_ref(&tail_text)),
            2,
            Assoc::After,
        )
        .unwrap();
        (
            children[0].id(),
            <XmlTextRef as AsRef<Branch>>::as_ref(&left_text).id(),
            children[1].id(),
            <XmlTextRef as AsRef<Branch>>::as_ref(&tail_text).id(),
            tail_sticky,
            txn.encode_state_as_update_v1(&StateVector::default()).len(),
        )
    };
    let mut compiled = {
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
                request_id: 120,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalCommand,
                operations: vec![TypedOperation::SplitBlock {
                    at: point_for_test(2),
                    node_type: "paragraph".into(),
                    attrs: HashMap::new(),
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
        compiled.mutation_plan.actions.as_slice(),
        [
            YrsMutationAction::DeleteText {
                index_utf16: 3,
                len_utf16: 1,
                operation_index: 0,
                ..
            },
            YrsMutationAction::InsertXmlChildren {
                child_index: 1,
                nodes,
                operation_index: 0,
                ..
            }
        ] if nodes.len() == 1 && nodes[0].index == 1
    ));
    {
        let txn = doc.transact();
        let preflight =
            preflight_mutation_work_for_test(120, &compiled.mutation_plan, &txn).unwrap();
        let exact = compiled.mutation_plan.compilation_work_for_test() + preflight;
        compiled.mutation_plan.set_work_limit_for_test(exact);
        preflight_mutation_plan(120, &compiled.mutation_plan, &txn).unwrap();
        compiled.mutation_plan.set_work_limit_for_test(exact - 1);
        assert_eq!(
            preflight_mutation_plan(120, &compiled.mutation_plan, &txn)
                .unwrap_err()
                .code,
            "OPERATION_LIMIT_EXCEEDED"
        );
        compiled.mutation_plan.set_work_limit_for_test(exact);
    }
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    let estimate = compiled.encoded_growth_bound;
    let update = {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
        txn.commit();
        txn.encode_update_v1()
    };
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let children = fragment.children(&txn).collect::<Vec<_>>();
    assert_eq!(children.len(), 3);
    assert_eq!(children[0].id(), left_block_id);
    assert_eq!(children[2].id(), tail_block_id);
    assert_ne!(children[1].id(), left_block_id);
    assert_ne!(children[1].id(), tail_block_id);
    assert_eq!(
        <XmlTextRef as AsRef<Branch>>::as_ref(&paragraph_text(&fragment, &txn, 0)).id(),
        left_text_id
    );
    assert_eq!(
        <XmlTextRef as AsRef<Branch>>::as_ref(&paragraph_text(&fragment, &txn, 2)).id(),
        tail_text_id
    );
    let resolved_sticky = tail_sticky.get_offset(&txn).unwrap();
    assert_eq!(resolved_sticky.branch.id(), tail_text_id);
    assert_eq!(resolved_sticky.index, 2);
    let actual = codec.read_json(&fragment, &txn).unwrap();
    assert_eq!(actual, expected);
    assert_eq!(actual["content"].as_array().unwrap().len(), 3);
    assert_eq!(actual["content"][0]["content"][0]["text"], "A😀");
    assert_eq!(actual["content"][1]["content"][0]["text"], "B");
    assert!(update.len() <= estimate, "{} > {estimate}", update.len());
    let after_full_len = txn.encode_state_as_update_v1(&StateVector::default()).len();
    assert!(after_full_len <= before_full_len + estimate);
}

#[test]
fn split_block_then_insert_text_targets_the_created_right_block() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "A😀B" }]
        }]
    });
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![
            TypedOperation::SplitBlock {
                at: point_for_test(2),
                node_type: "paragraph".into(),
                attrs: HashMap::new(),
            },
            TypedOperation::InsertText {
                at: point_for_test(2),
                text: "R".into(),
                marks: vec![],
            },
        ],
    );
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][1]["content"][0]["text"], "RB");
}

#[test]
fn insert_text_then_split_block_folds_into_the_retained_left_text() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "A😀B" }]
        }]
    });
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![
            TypedOperation::InsertText {
                at: point_for_test(2),
                text: "L".into(),
                marks: vec![],
            },
            TypedOperation::SplitBlock {
                at: point_for_test(2),
                node_type: "paragraph".into(),
                attrs: HashMap::new(),
            },
        ],
    );
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][0]["content"][0]["text"], "A😀L");
    assert_eq!(actual["content"][1]["content"][0]["text"], "B");
}

#[test]
fn insert_text_in_copied_suffix_then_split_folds_into_the_prepared_right_block() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "A😀B" }]
        }]
    });
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![
            TypedOperation::InsertText {
                at: point_for_test(3),
                text: "X".into(),
                marks: vec![],
            },
            TypedOperation::SplitBlock {
                at: point_for_test(2),
                node_type: "paragraph".into(),
                attrs: HashMap::new(),
            },
        ],
    );
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][0]["content"][0]["text"], "A😀");
    assert_eq!(actual["content"][1]["content"][0]["text"], "BX");
}

#[test]
fn mark_in_copied_suffix_then_split_folds_into_the_prepared_right_block() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "A😀B" }]
        }]
    });
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![
            TypedOperation::AddMark {
                range: range_for_test(2, 3),
                mark: Mark::new("bold".into(), HashMap::new()),
            },
            TypedOperation::SplitBlock {
                at: point_for_test(2),
                node_type: "paragraph".into(),
                attrs: HashMap::new(),
            },
        ],
    );
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][1]["content"][0]["text"], "B");
    assert_eq!(
        actual["content"][1]["content"][0]["marks"][0]["type"],
        "bold"
    );
}

#[test]
fn split_block_same_boundary_affinity_selects_left_or_right_block() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "A😀B" }]
        }]
    });
    let operations = |affinity| {
        vec![
            TypedOperation::SplitBlock {
                at: point_for_test(2),
                node_type: "paragraph".into(),
                attrs: HashMap::new(),
            },
            TypedOperation::InsertText {
                at: RevisionedPosition {
                    offset: 2,
                    kind: EditorOffsetKind::Scalar,
                    affinity,
                },
                text: "X".into(),
                marks: vec![],
            },
        ]
    };
    let (before, expected_before, _, _, _) =
        compile_and_execute(source.clone(), operations(Affinity::Before));
    assert_eq!(before, expected_before);
    assert_eq!(before["content"][0]["content"][0]["text"], "A😀X");
    assert_eq!(before["content"][1]["content"][0]["text"], "B");

    let (after, expected_after, _, _, _) = compile_and_execute(source, operations(Affinity::After));
    assert_eq!(after, expected_after);
    assert_eq!(after["content"][0]["content"][0]["text"], "A😀");
    assert_eq!(after["content"][1]["content"][0]["text"], "XB");
}

#[test]
fn split_block_then_update_attrs_mutates_the_prepared_right_block() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "h2",
            "attrs": { "id": "left" },
            "content": [{ "type": "text", "text": "abcd" }]
        }]
    });
    let schema = attribute_schema();
    let (doc, schema, limits, editing_limits, document) =
        diagnostic_doc_with_schema(&source, schema);
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
                request_id: 121,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalCommand,
                operations: vec![
                    TypedOperation::SplitBlock {
                        at: point_for_test(2),
                        node_type: "h2".into(),
                        attrs: HashMap::from([("id".into(), Value::String("right-old".into()))]),
                    },
                    TypedOperation::UpdateNodeAttrs {
                        at: point_for_test(2),
                        attrs: HashMap::from([("id".into(), Value::String("right-new".into()))]),
                    },
                ],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
        .unwrap()
    };
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let actual = YrsDocumentCodec::new(&schema, &limits)
        .read_json(&fragment, &txn)
        .unwrap();
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][0]["attrs"]["id"], "left");
    assert_eq!(actual["content"][1]["attrs"]["id"], "right-new");
}

#[test]
fn split_block_immediately_before_and_after_an_atom_moves_only_the_suffix_children() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [
                { "type": "text", "text": "A" },
                { "type": "hardBreak" },
                { "type": "text", "text": "B" }
            ]
        }]
    });
    for (offset, retained_children, right_first_type) in
        [(1, 1usize, "hardBreak"), (2, 2usize, "text")]
    {
        let (doc, schema, limits, compiled) = compile_operations_with_schema(
            &source,
            vec![TypedOperation::SplitBlock {
                at: point_for_test(offset),
                node_type: "paragraph".into(),
                attrs: HashMap::new(),
            }],
            tiptap_schema(),
        );
        let (left_block_id, left_child_ids, before_full_len) = {
            let txn = doc.transact();
            let fragment = txn.get_xml_fragment("prosemirror").unwrap();
            let XmlOut::Element(left) = fragment.get(&txn, 0).unwrap() else {
                panic!("left paragraph expected")
            };
            (
                <yrs::types::xml::XmlElementRef as AsRef<Branch>>::as_ref(&left).id(),
                left.children(&txn)
                    .map(|child| child.id())
                    .collect::<Vec<_>>(),
                txn.encode_state_as_update_v1(&StateVector::default()).len(),
            )
        };
        assert!(matches!(
            compiled.mutation_plan.actions.as_slice(),
            [
                YrsMutationAction::DeleteXmlChildren { .. },
                YrsMutationAction::InsertXmlChildren { .. }
            ]
        ));
        let expected = to_prosemirror_json(&compiled.preview, &schema);
        let estimate = compiled.encoded_growth_bound;
        let update = {
            let mut txn = doc.transact_mut();
            execute_mutation_plan(compiled.mutation_plan, &mut txn);
            txn.commit();
            txn.encode_update_v1()
        };
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let XmlOut::Element(left) = fragment.get(&txn, 0).unwrap() else {
            panic!("retained paragraph expected")
        };
        assert_eq!(
            <yrs::types::xml::XmlElementRef as AsRef<Branch>>::as_ref(&left).id(),
            left_block_id
        );
        let after_left_ids = left
            .children(&txn)
            .map(|child| child.id())
            .collect::<Vec<_>>();
        assert_eq!(after_left_ids, left_child_ids[..retained_children]);
        let XmlOut::Element(right) = fragment.get(&txn, 1).unwrap() else {
            panic!("prepared right paragraph expected")
        };
        let right_ids = right
            .children(&txn)
            .map(|child| child.id())
            .collect::<Vec<_>>();
        assert!(right_ids.iter().all(|id| !left_child_ids.contains(id)));
        let actual = YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap();
        assert_eq!(actual, expected);
        assert_eq!(actual["content"][1]["content"][0]["type"], right_first_type);
        assert!(update.len() <= estimate);
        let after_full_len = txn.encode_state_as_update_v1(&StateVector::default()).len();
        assert!(after_full_len <= before_full_len + estimate);
    }
}

#[test]
fn split_atom_boundary_builds_canonical_h2_and_code_block_blueprints_with_follow_up_edits() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [
                { "type": "text", "text": "A" },
                { "type": "hardBreak" },
                { "type": "text", "text": "B" }
            ]
        }]
    });
    for (node_type, attrs, expected_type) in [
        (
            "h2",
            HashMap::from([("id".into(), Value::String("right".into()))]),
            "h2",
        ),
        (
            "codeBlock",
            HashMap::from([("language".into(), Value::String("rust".into()))]),
            "codeBlock",
        ),
    ] {
        let (doc, schema, limits, compiled) = compile_operations_with_schema(
            &source,
            vec![
                TypedOperation::SplitBlock {
                    at: point_for_test(2),
                    node_type: node_type.into(),
                    attrs,
                },
                TypedOperation::InsertText {
                    at: point_for_test(2),
                    text: "X".into(),
                    marks: vec![],
                },
            ],
            attribute_schema(),
        );
        let expected = to_prosemirror_json(&compiled.preview, &schema);
        {
            let mut txn = doc.transact_mut();
            execute_mutation_plan(compiled.mutation_plan, &mut txn);
        }
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let actual = YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap();
        assert_eq!(actual, expected);
        assert_eq!(actual["content"][1]["type"], expected_type);
        assert_eq!(actual["content"][1]["content"][0]["text"], "XB");
    }
}

#[test]
fn split_atom_boundary_accounts_for_multiple_atoms_and_no_preceding_text() {
    let multiple = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [
                { "type": "text", "text": "A" },
                { "type": "hardBreak" },
                { "type": "hardBreak" },
                { "type": "text", "text": "B" }
            ]
        }]
    });
    let (actual, expected, _, _, _) = compile_and_execute(
        multiple,
        vec![
            TypedOperation::SplitBlock {
                at: point_for_test(3),
                node_type: "paragraph".into(),
                attrs: HashMap::new(),
            },
            TypedOperation::InsertText {
                at: point_for_test(3),
                text: "X".into(),
                marks: vec![],
            },
        ],
    );
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][1]["content"][0]["text"], "XB");

    let no_prefix = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [
                { "type": "hardBreak" },
                { "type": "text", "text": "B" }
            ]
        }]
    });
    let (actual, expected, _, _, _) = compile_and_execute(
        no_prefix,
        vec![TypedOperation::SplitBlock {
            at: point_for_test(0),
            node_type: "paragraph".into(),
            attrs: HashMap::new(),
        }],
    );
    assert_eq!(actual, expected);
    assert!(actual["content"][0].get("content").is_none());
    assert_eq!(actual["content"][1]["content"][0]["type"], "hardBreak");
    assert_eq!(actual["content"][1]["content"][1]["text"], "B");
}

#[test]
fn split_block_inside_list_item_inserts_a_new_right_list_item() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [
                {
                    "type": "listItem",
                    "content": [
                        {
                            "type": "paragraph",
                            "content": [{ "type": "text", "text": "A😀B" }]
                        },
                        {
                            "type": "paragraph",
                            "content": [{ "type": "text", "text": "post" }]
                        }
                    ]
                },
                {
                    "type": "listItem",
                    "content": [{
                        "type": "paragraph",
                        "content": [{ "type": "text", "text": "tail" }]
                    }]
                }
            ]
        }]
    });
    let schema = tiptap_schema();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&document, &schema);
    let byte = rendered.find("A😀B").unwrap();
    let offset = u32::try_from(rendered[..byte].chars().count() + 2).unwrap();
    let (doc, schema, limits, mut compiled) = compile_operations_with_schema(
        &source,
        vec![TypedOperation::SplitBlock {
            at: point_for_test(offset),
            node_type: "paragraph".into(),
            attrs: HashMap::new(),
        }],
        tiptap_schema(),
    );
    assert!(matches!(
        compiled.mutation_plan.actions.as_slice(),
        [
            YrsMutationAction::DeleteText {
                index_utf16: 3,
                len_utf16: 1,
                operation_index: 0,
                ..
            },
            YrsMutationAction::DeleteXmlChildren {
                child_index: 1,
                child_count: 1,
                operation_index: 0,
                ..
            },
            YrsMutationAction::InsertXmlChildren {
                child_index: 1,
                nodes,
                operation_index: 0,
                ..
            }
        ] if nodes.len() == 1 && nodes[0].index == 1
    ));
    let (
        list_id,
        first_item_id,
        first_paragraph_id,
        first_text_id,
        post_paragraph_id,
        tail_item_id,
        tail_paragraph_id,
        tail_text_id,
        tail_sticky,
        before_full_len,
    ) = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let XmlOut::Element(list) = fragment.get(&txn, 0).unwrap() else {
            panic!("list expected")
        };
        let XmlOut::Element(first_item) = list.get(&txn, 0).unwrap() else {
            panic!("first list item expected")
        };
        let XmlOut::Element(first_paragraph) = first_item.get(&txn, 0).unwrap() else {
            panic!("first paragraph expected")
        };
        let XmlOut::Text(first_text) = first_paragraph.get(&txn, 0).unwrap() else {
            panic!("first text expected")
        };
        let post_paragraph_id = first_item.get(&txn, 1).unwrap().id();
        let XmlOut::Element(tail_item) = list.get(&txn, 1).unwrap() else {
            panic!("tail list item expected")
        };
        let XmlOut::Element(tail_paragraph) = tail_item.get(&txn, 0).unwrap() else {
            panic!("tail paragraph expected")
        };
        let XmlOut::Text(tail_text) = tail_paragraph.get(&txn, 0).unwrap() else {
            panic!("tail text expected")
        };
        let tail_sticky = StickyIndex::at(
            &txn,
            BranchPtr::from(<XmlTextRef as AsRef<Branch>>::as_ref(&tail_text)),
            2,
            Assoc::After,
        )
        .unwrap();
        (
            AsRef::<Branch>::as_ref(&list).id(),
            AsRef::<Branch>::as_ref(&first_item).id(),
            AsRef::<Branch>::as_ref(&first_paragraph).id(),
            <XmlTextRef as AsRef<Branch>>::as_ref(&first_text).id(),
            post_paragraph_id,
            AsRef::<Branch>::as_ref(&tail_item).id(),
            AsRef::<Branch>::as_ref(&tail_paragraph).id(),
            <XmlTextRef as AsRef<Branch>>::as_ref(&tail_text).id(),
            tail_sticky,
            txn.encode_state_as_update_v1(&StateVector::default()).len(),
        )
    };
    {
        let txn = doc.transact();
        let preflight =
            preflight_mutation_work_for_test(122, &compiled.mutation_plan, &txn).unwrap();
        let exact = compiled.mutation_plan.compilation_work_for_test() + preflight;
        compiled.mutation_plan.set_work_limit_for_test(exact);
        preflight_mutation_plan(122, &compiled.mutation_plan, &txn).unwrap();
        compiled.mutation_plan.set_work_limit_for_test(exact - 1);
        assert_eq!(
            preflight_mutation_plan(122, &compiled.mutation_plan, &txn)
                .unwrap_err()
                .code,
            "OPERATION_LIMIT_EXCEEDED"
        );
        compiled.mutation_plan.set_work_limit_for_test(exact);
    }
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    let estimate = compiled.encoded_growth_bound;
    let update = {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
        txn.commit();
        txn.encode_update_v1()
    };
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let actual = YrsDocumentCodec::new(&schema, &limits)
        .read_json(&fragment, &txn)
        .unwrap();
    assert_eq!(actual, expected);
    let items = actual["content"][0]["content"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(items[0]["content"][0]["content"][0]["text"], "A😀");
    assert_eq!(items[1]["content"][0]["content"][0]["text"], "B");
    assert_eq!(items[1]["content"][1]["content"][0]["text"], "post");
    assert_eq!(items[2]["content"][0]["content"][0]["text"], "tail");

    let XmlOut::Element(list) = fragment.get(&txn, 0).unwrap() else {
        panic!("list expected")
    };
    assert_eq!(AsRef::<Branch>::as_ref(&list).id(), list_id);
    let XmlOut::Element(first_item) = list.get(&txn, 0).unwrap() else {
        panic!("first list item expected")
    };
    let XmlOut::Element(new_item) = list.get(&txn, 1).unwrap() else {
        panic!("new list item expected")
    };
    let XmlOut::Element(tail_item) = list.get(&txn, 2).unwrap() else {
        panic!("tail list item expected")
    };
    assert_eq!(AsRef::<Branch>::as_ref(&first_item).id(), first_item_id);
    assert_ne!(AsRef::<Branch>::as_ref(&new_item).id(), first_item_id);
    assert_ne!(AsRef::<Branch>::as_ref(&new_item).id(), tail_item_id);
    assert_eq!(AsRef::<Branch>::as_ref(&tail_item).id(), tail_item_id);
    let XmlOut::Element(first_paragraph) = first_item.get(&txn, 0).unwrap() else {
        panic!("retained first paragraph expected")
    };
    let XmlOut::Text(first_text) = first_paragraph.get(&txn, 0).unwrap() else {
        panic!("retained first text expected")
    };
    assert_eq!(
        AsRef::<Branch>::as_ref(&first_paragraph).id(),
        first_paragraph_id
    );
    assert_eq!(
        <XmlTextRef as AsRef<Branch>>::as_ref(&first_text).id(),
        first_text_id
    );
    assert_ne!(new_item.get(&txn, 1).unwrap().id(), post_paragraph_id);
    let XmlOut::Element(tail_paragraph) = tail_item.get(&txn, 0).unwrap() else {
        panic!("retained tail paragraph expected")
    };
    let XmlOut::Text(tail_text) = tail_paragraph.get(&txn, 0).unwrap() else {
        panic!("retained tail text expected")
    };
    assert_eq!(
        AsRef::<Branch>::as_ref(&tail_paragraph).id(),
        tail_paragraph_id
    );
    assert_eq!(
        <XmlTextRef as AsRef<Branch>>::as_ref(&tail_text).id(),
        tail_text_id
    );
    let resolved_sticky = tail_sticky.get_offset(&txn).unwrap();
    assert_eq!(resolved_sticky.branch.id(), tail_text_id);
    assert_eq!(resolved_sticky.index, 2);
    assert!(update.len() <= estimate, "{} > {estimate}", update.len());
    let after_full_len = txn.encode_state_as_update_v1(&StateVector::default()).len();
    assert!(after_full_len <= before_full_len + estimate);
}

#[test]
fn cross_parent_delete_merges_marked_unicode_paragraph_suffix_directly() {
    let source = json!({
        "type": "doc",
        "content": [
            {
                "type": "paragraph",
                "content": [{
                    "type": "text",
                    "text": "A😀B",
                    "marks": [{ "type": "bold" }]
                }]
            },
            {
                "type": "paragraph",
                "content": [{
                    "type": "text",
                    "text": "C😀D",
                    "marks": [{ "type": "bold" }]
                }]
            },
            {
                "type": "paragraph",
                "content": [{ "type": "text", "text": "tail" }]
            }
        ]
    });
    let schema = tiptap_schema();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&document, &schema);
    let first_byte = rendered.find("A😀B").unwrap();
    let second_byte = rendered.find("C😀D").unwrap();
    let from = u32::try_from(rendered[..first_byte].chars().count() + 2).unwrap();
    let to = u32::try_from(rendered[..second_byte].chars().count() + 2).unwrap();
    let operations = || {
        vec![TypedOperation::DeleteRange {
            range: range_for_test(from, to),
        }]
    };
    let (doc, schema, limits, mut compiled) =
        compile_operations_with_schema(&source, operations(), tiptap_schema());
    assert!(matches!(
        compiled.mutation_plan.actions.as_slice(),
        [
            YrsMutationAction::DeleteText {
                index_utf16: 3,
                len_utf16: 1,
                operation_index: 0,
                ..
            },
            YrsMutationAction::InsertText {
                index_utf16: 3,
                text,
                len_utf16: 1,
                operation_index: 0,
                ..
            },
            YrsMutationAction::DeleteXmlChildren {
                child_index: 1,
                child_count: 1,
                operation_index: 0,
                ..
            }
        ] if text == "D"
    ));
    let (
        first_block_id,
        first_text_id,
        removed_block_id,
        tail_block_id,
        tail_text_id,
        tail_sticky,
        before_full_len,
        before_update,
    ) = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let children = fragment.children(&txn).collect::<Vec<_>>();
        let first_text = paragraph_text(&fragment, &txn, 0);
        let tail_text = paragraph_text(&fragment, &txn, 2);
        let tail_sticky = StickyIndex::at(
            &txn,
            BranchPtr::from(<XmlTextRef as AsRef<Branch>>::as_ref(&tail_text)),
            2,
            Assoc::After,
        )
        .unwrap();
        (
            children[0].id(),
            <XmlTextRef as AsRef<Branch>>::as_ref(&first_text).id(),
            children[1].id(),
            children[2].id(),
            <XmlTextRef as AsRef<Branch>>::as_ref(&tail_text).id(),
            tail_sticky,
            txn.encode_state_as_update_v1(&StateVector::default()).len(),
            txn.encode_state_as_update_v1(&StateVector::default()),
        )
    };
    {
        let txn = doc.transact();
        let preflight =
            preflight_mutation_work_for_test(122, &compiled.mutation_plan, &txn).unwrap();
        let exact = compiled.mutation_plan.compilation_work_for_test() + preflight;
        compiled.mutation_plan.set_work_limit_for_test(exact);
        preflight_mutation_plan(122, &compiled.mutation_plan, &txn).unwrap();
        compiled.mutation_plan.set_work_limit_for_test(exact - 1);
        assert_eq!(
            preflight_mutation_plan(122, &compiled.mutation_plan, &txn)
                .unwrap_err()
                .code,
            "OPERATION_LIMIT_EXCEEDED"
        );
        compiled.mutation_plan.set_work_limit_for_test(exact);
    }
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    let estimate = compiled.encoded_growth_bound;
    let undo_exact = compiled.undo_units_bound;
    let update = {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
        txn.commit();
        txn.encode_update_v1()
    };
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let actual = YrsDocumentCodec::new(&schema, &limits)
        .read_json(&fragment, &txn)
        .unwrap();
    assert_eq!(actual, expected);
    assert_eq!(actual["content"].as_array().unwrap().len(), 2);
    assert_eq!(actual["content"][0]["content"][0]["text"], "A😀D");
    assert_eq!(
        actual["content"][0]["content"][0]["marks"][0]["type"],
        "bold"
    );
    assert_eq!(actual["content"][1]["content"][0]["text"], "tail");
    let children = fragment.children(&txn).collect::<Vec<_>>();
    assert_eq!(children[0].id(), first_block_id);
    assert_eq!(children[1].id(), tail_block_id);
    assert!(!children.iter().any(|child| child.id() == removed_block_id));
    assert_eq!(
        <XmlTextRef as AsRef<Branch>>::as_ref(&paragraph_text(&fragment, &txn, 0)).id(),
        first_text_id
    );
    assert_eq!(
        <XmlTextRef as AsRef<Branch>>::as_ref(&paragraph_text(&fragment, &txn, 1)).id(),
        tail_text_id
    );
    let resolved_sticky = tail_sticky.get_offset(&txn).unwrap();
    assert_eq!(resolved_sticky.branch.id(), tail_text_id);
    assert_eq!(resolved_sticky.index, 2);
    assert!(update.len() <= estimate, "{} > {estimate}", update.len());
    let after_full_len = txn.encode_state_as_update_v1(&StateVector::default()).len();
    assert!(after_full_len <= before_full_len + estimate);
    let replica = utf16_doc();
    {
        let mut replica_txn = replica.transact_mut();
        replica_txn
            .apply_update(Update::decode_v1(&before_update).unwrap())
            .unwrap();
        replica_txn
            .apply_update(Update::decode_v1(&update).unwrap())
            .unwrap();
    }
    let replica_txn = replica.transact();
    let replica_fragment = replica_txn.get_xml_fragment("prosemirror").unwrap();
    assert_eq!(
        YrsDocumentCodec::new(&schema, &limits)
            .read_json(&replica_fragment, &replica_txn)
            .unwrap(),
        expected
    );

    assert!(undo_exact > 0);
    assert_eq!(
        compile_operations_with_undo_limit(&source, operations(), tiptap_schema(), undo_exact,)
            .unwrap()
            .undo_units_bound,
        undo_exact
    );
    let undo_error =
        compile_operations_with_undo_limit(&source, operations(), tiptap_schema(), undo_exact - 1)
            .unwrap_err();
    assert_eq!(undo_error.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(undo_error.limit, Some(undo_exact - 1));
    assert_eq!(undo_error.actual, Some(undo_exact));
}

#[test]
fn cross_parent_delete_uses_provenance_across_equal_blocks_and_a_middle_void() {
    let source = json!({
        "type": "doc",
        "content": [
            {
                "type": "paragraph",
                "content": [{
                    "type": "text",
                    "text": "A😀B",
                    "marks": [{ "type": "bold" }]
                }]
            },
            { "type": "horizontalRule" },
            {
                "type": "paragraph",
                "content": [{
                    "type": "text",
                    "text": "A😀B",
                    "marks": [{ "type": "bold" }]
                }]
            }
        ]
    });
    let schema = tiptap_schema();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&document, &schema);
    let occurrences = rendered
        .match_indices("A😀B")
        .map(|(byte, _)| byte)
        .collect::<Vec<_>>();
    let from = u32::try_from(rendered[..occurrences[0]].chars().count() + 1).unwrap();
    let to = u32::try_from(rendered[..occurrences[1]].chars().count() + 2).unwrap();
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![TypedOperation::DeleteRange {
            range: range_for_test(from, to),
        }],
        tiptap_schema(),
    );
    assert!(matches!(
        compiled.mutation_plan.actions.as_slice(),
        [
            YrsMutationAction::DeleteText {
                index_utf16: 1,
                len_utf16: 3,
                ..
            },
            YrsMutationAction::InsertText {
                index_utf16: 1,
                text,
                ..
            },
            YrsMutationAction::DeleteXmlChildren {
                child_index: 1,
                child_count: 2,
                ..
            }
        ] if text == "B"
    ));
    let first_id = {
        let txn = doc.transact();
        txn.get_xml_fragment("prosemirror")
            .unwrap()
            .get(&txn, 0)
            .unwrap()
            .id()
    };
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    assert_eq!(fragment.get(&txn, 0).unwrap().id(), first_id);
    let actual = YrsDocumentCodec::new(&schema, &limits)
        .read_json(&fragment, &txn)
        .unwrap();
    assert_eq!(actual, expected);
    assert_eq!(actual["content"].as_array().unwrap().len(), 1);
    assert_eq!(actual["content"][0]["content"][0]["text"], "AB");
}

#[test]
fn cross_parent_replace_inserts_inline_text_and_atom_fragment_directly() {
    let source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "ab" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "cd" }] }
        ]
    });
    let schema = tiptap_schema();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&document, &schema);
    let first = rendered.find("ab").unwrap();
    let second = rendered.find("cd").unwrap();
    let from = u32::try_from(rendered[..first].chars().count() + 1).unwrap();
    let to = u32::try_from(rendered[..second].chars().count() + 1).unwrap();
    let replacement = || {
        Fragment::from(vec![
            Node::text("X".into(), vec![Mark::new("bold".into(), HashMap::new())]),
            Node::void("hardBreak".into(), HashMap::new()),
            Node::text("Y".into(), vec![]),
        ])
    };
    let operations = || {
        vec![TypedOperation::ReplaceRange {
            range: range_for_test(from, to),
            content: replacement(),
        }]
    };
    let (doc, schema, limits, mut compiled) =
        compile_operations_with_schema(&source, operations(), tiptap_schema());
    assert!(matches!(
        compiled.mutation_plan.actions.as_slice(),
        [
            YrsMutationAction::DeleteXmlChildren {
                child_index: 1,
                child_count: 1,
                operation_index: 0,
                ..
            },
            YrsMutationAction::DeleteText {
                index_utf16: 1,
                len_utf16: 1,
                operation_index: 0,
                ..
            },
            YrsMutationAction::InsertText {
                index_utf16: 1,
                text,
                len_utf16: 1,
                operation_index: 0,
                ..
            },
            YrsMutationAction::InsertXmlChildren {
                child_index: 1,
                nodes,
                operation_index: 0,
                ..
            }
        ] if text == "X"
            && matches!(nodes.as_slice(), [
                PreparedXmlChild { index: 1, node: PreparedXmlNode::Element { tag, .. } },
                PreparedXmlChild { index: 2, node: PreparedXmlNode::Text { runs } }
            ] if tag == "hardBreak" && prepared_text_for_test(runs) == "Yd")
    ));
    let (first_block_id, first_text_id, right_text_id, before_full_len) = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let first_text = paragraph_text(&fragment, &txn, 0);
        let right_text = paragraph_text(&fragment, &txn, 1);
        (
            fragment.get(&txn, 0).unwrap().id(),
            <XmlTextRef as AsRef<Branch>>::as_ref(&first_text).id(),
            <XmlTextRef as AsRef<Branch>>::as_ref(&right_text).id(),
            txn.encode_state_as_update_v1(&StateVector::default()).len(),
        )
    };
    assert!(compiled
        .mutation_plan
        .actions
        .iter()
        .all(|action| match action {
            YrsMutationAction::InsertText { target, .. }
            | YrsMutationAction::DeleteText { target, .. }
            | YrsMutationAction::FormatText { target, .. } =>
                AsRef::<Branch>::as_ref(target).id() != right_text_id,
            _ => true,
        }));
    {
        let txn = doc.transact();
        let preflight =
            preflight_mutation_work_for_test(122, &compiled.mutation_plan, &txn).unwrap();
        let exact = compiled.mutation_plan.compilation_work_for_test() + preflight;
        compiled.mutation_plan.set_work_limit_for_test(exact);
        preflight_mutation_plan(122, &compiled.mutation_plan, &txn).unwrap();
        compiled.mutation_plan.set_work_limit_for_test(exact - 1);
        assert_eq!(
            preflight_mutation_plan(122, &compiled.mutation_plan, &txn)
                .unwrap_err()
                .code,
            "OPERATION_LIMIT_EXCEEDED"
        );
        compiled.mutation_plan.set_work_limit_for_test(exact);
    }
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    let estimate = compiled.encoded_growth_bound;
    let undo_exact = compiled.undo_units_bound;
    let update = {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
        txn.commit();
        txn.encode_update_v1()
    };
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let actual = YrsDocumentCodec::new(&schema, &limits)
        .read_json(&fragment, &txn)
        .unwrap();
    assert_eq!(actual, expected);
    let content = actual["content"][0]["content"].as_array().unwrap();
    assert_eq!(content[0]["text"], "a");
    assert_eq!(content[1]["text"], "X");
    assert_eq!(content[1]["marks"][0]["type"], "bold");
    assert_eq!(content[2]["type"], "hardBreak");
    assert_eq!(content[3]["text"], "Yd");
    assert_eq!(fragment.get(&txn, 0).unwrap().id(), first_block_id);
    assert_eq!(
        <XmlTextRef as AsRef<Branch>>::as_ref(&paragraph_text(&fragment, &txn, 0)).id(),
        first_text_id
    );
    assert!(update.len() <= estimate, "{} > {estimate}", update.len());
    let after_full_len = txn.encode_state_as_update_v1(&StateVector::default()).len();
    assert!(after_full_len <= before_full_len + estimate);
    assert_eq!(
        compile_operations_with_undo_limit(&source, operations(), tiptap_schema(), undo_exact,)
            .unwrap()
            .undo_units_bound,
        undo_exact
    );
    let undo_error =
        compile_operations_with_undo_limit(&source, operations(), tiptap_schema(), undo_exact - 1)
            .unwrap_err();
    assert_eq!(undo_error.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(undo_error.actual, Some(undo_exact));
}

#[test]
fn cross_parent_replace_handles_empty_text_only_leading_and_multiple_atoms() {
    let source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "ab" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "cd" }] }
        ]
    });
    let schema = tiptap_schema();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&document, &schema);
    let first = rendered.find("ab").unwrap();
    let second = rendered.find("cd").unwrap();
    let from = u32::try_from(rendered[..first].chars().count() + 1).unwrap();
    let to = u32::try_from(rendered[..second].chars().count() + 1).unwrap();
    let cases = vec![
        (Fragment::empty(), "ad", 1usize),
        (
            Fragment::from(vec![Node::text("Q".into(), vec![])]),
            "aQd",
            1,
        ),
        (
            Fragment::from(vec![
                Node::void("hardBreak".into(), HashMap::new()),
                Node::text("Y".into(), vec![]),
            ]),
            "aYd",
            3,
        ),
        (
            Fragment::from(vec![
                Node::void("hardBreak".into(), HashMap::new()),
                Node::text("X".into(), vec![]),
                Node::void("hardBreak".into(), HashMap::new()),
                Node::text("Y".into(), vec![]),
            ]),
            "aXYd",
            5,
        ),
    ];
    for (replacement, expected_text, expected_children) in cases {
        let (actual, expected, _, _, _) = compile_and_execute(
            source.clone(),
            vec![TypedOperation::ReplaceRange {
                range: range_for_test(from, to),
                content: replacement,
            }],
        );
        assert_eq!(actual, expected);
        assert_eq!(
            actual["content"][0]["content"].as_array().unwrap().len(),
            expected_children
        );
        let decoded = from_prosemirror_json(&actual, &schema, UnknownTypeMode::Preserve).unwrap();
        assert_eq!(decoded.root().text_content(), expected_text);
    }
}

#[test]
fn cross_parent_replace_folds_follow_up_edits_into_prepared_children() {
    let source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "ab" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "cd" }] }
        ]
    });
    let schema = tiptap_schema();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&document, &schema);
    let first = rendered.find("ab").unwrap();
    let second = rendered.find("cd").unwrap();
    let from = u32::try_from(rendered[..first].chars().count() + 1).unwrap();
    let to = u32::try_from(rendered[..second].chars().count() + 1).unwrap();
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::ReplaceRange {
                range: range_for_test(from, to),
                content: Fragment::from(vec![
                    Node::void("hardBreak".into(), HashMap::new()),
                    Node::text("Y".into(), vec![]),
                ]),
            },
            TypedOperation::InsertText {
                at: point_for_test(to),
                text: "Z".into(),
                marks: vec![],
            },
        ],
        tiptap_schema(),
    );
    assert!(
        !compiled.mutation_plan.actions.iter().any(|action| matches!(
            action,
            YrsMutationAction::InsertText {
                operation_index: 1,
                ..
            }
        ))
    );
    let prepared_text = compiled
        .mutation_plan
        .actions
        .iter()
        .find_map(|action| match action {
            YrsMutationAction::InsertXmlChildren { nodes, .. } => nodes.last(),
            _ => None,
        })
        .and_then(|child| match &child.node {
            PreparedXmlNode::Text { runs } => Some(prepared_text_for_test(runs)),
            _ => None,
        })
        .unwrap();
    assert!(prepared_text.contains('Z'));
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    assert_eq!(
        YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap(),
        expected
    );
}

#[test]
fn cross_parent_replace_updates_a_prepared_inline_atom_blueprint() {
    let source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "ab" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "cd" }] }
        ]
    });
    let schema = attribute_schema();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&document, &schema);
    let first = rendered.find("ab").unwrap();
    let second = rendered.find("cd").unwrap();
    let from = u32::try_from(rendered[..first].chars().count() + 1).unwrap();
    let to = u32::try_from(rendered[..second].chars().count() + 1).unwrap();
    let atom_position = RevisionedPosition {
        offset: from,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::Before,
    };
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::ReplaceRange {
                range: range_for_test(from, to),
                content: Fragment::from(vec![Node::void(
                    "inlineWidget".into(),
                    HashMap::from([
                        ("id".into(), Value::String("old".into())),
                        ("meta".into(), json!({ "nested": [1, true] })),
                    ]),
                )]),
            },
            TypedOperation::UpdateNodeAttrs {
                at: atom_position,
                attrs: HashMap::from([
                    ("id".into(), Value::String("new".into())),
                    ("meta".into(), json!({ "nested": [2, false] })),
                ]),
            },
        ],
        schema,
    );
    assert!(!compiled.mutation_plan.actions.iter().any(|action| {
        matches!(
            action,
            YrsMutationAction::SetXmlAttribute {
                operation_index: 1,
                ..
            } | YrsMutationAction::RemoveXmlAttribute {
                operation_index: 1,
                ..
            }
        )
    }));
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let actual = YrsDocumentCodec::new(&schema, &limits)
        .read_json(&fragment, &txn)
        .unwrap();
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][0]["content"][1]["attrs"]["id"], "new");
    assert_eq!(
        actual["content"][0]["content"][1]["attrs"]["meta"],
        json!({ "nested": [2, false] })
    );
}

#[test]
fn cross_parent_replace_uses_virtual_runs_after_prior_endpoint_edits() {
    let source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "abcd" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "wxyz" }] }
        ]
    });
    let schema = tiptap_schema();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&document, &schema);
    let first = u32::try_from(rendered[..rendered.find("abcd").unwrap()].chars().count()).unwrap();
    let second = u32::try_from(rendered[..rendered.find("wxyz").unwrap()].chars().count()).unwrap();
    let replacement = || TypedOperation::ReplaceRange {
        range: range_for_test(first + 2, second + 2),
        content: Fragment::from(vec![
            Node::void("hardBreak".into(), HashMap::new()),
            Node::text("Q".into(), vec![]),
        ]),
    };
    let bold = || Mark::new("bold".into(), HashMap::new());
    let cases = vec![
        vec![
            TypedOperation::InsertText {
                at: point_for_test(first + 1),
                text: "L".into(),
                marks: vec![],
            },
            replacement(),
        ],
        vec![
            TypedOperation::InsertText {
                at: point_for_test(second + 1),
                text: "R".into(),
                marks: vec![],
            },
            replacement(),
        ],
        vec![
            TypedOperation::DeleteRange {
                range: range_for_test(first, first + 1),
            },
            replacement(),
        ],
        vec![
            TypedOperation::DeleteRange {
                range: range_for_test(second + 2, second + 3),
            },
            replacement(),
        ],
        vec![
            TypedOperation::AddMark {
                range: range_for_test(first, first + 1),
                mark: bold(),
            },
            replacement(),
        ],
        vec![
            TypedOperation::AddMark {
                range: range_for_test(second + 2, second + 3),
                mark: bold(),
            },
            replacement(),
        ],
    ];
    for operations in cases {
        let (actual, expected, _, _, _) = compile_and_execute(source.clone(), operations);
        assert_eq!(actual, expected);
        assert_eq!(actual["content"].as_array().unwrap().len(), 1);
        assert!(actual["content"][0]["content"]
            .as_array()
            .unwrap()
            .iter()
            .any(|node| node["type"] == "hardBreak"));
    }
}

#[test]
fn cross_parent_replace_uses_provenance_and_a_nested_lca() {
    let equal = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "same" }] },
            { "type": "horizontalRule" },
            { "type": "paragraph", "content": [{ "type": "text", "text": "same" }] }
        ]
    });
    let schema = tiptap_schema();
    let document = from_prosemirror_json(&equal, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&document, &schema);
    let occurrences = rendered
        .match_indices("same")
        .map(|(byte, _)| byte)
        .collect::<Vec<_>>();
    let from = u32::try_from(rendered[..occurrences[0]].chars().count() + 1).unwrap();
    let to = u32::try_from(rendered[..occurrences[1]].chars().count() + 1).unwrap();
    let (actual, expected, _, _, _) = compile_and_execute(
        equal,
        vec![TypedOperation::ReplaceRange {
            range: range_for_test(from, to),
            content: Fragment::from(vec![Node::text("X".into(), vec![])]),
        }],
    );
    assert_eq!(actual, expected);
    assert_eq!(actual["content"].as_array().unwrap().len(), 1);
    assert_eq!(actual["content"][0]["content"][0]["text"], "sXame");

    let nested = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [{
                "type": "listItem",
                "content": [
                    { "type": "paragraph", "content": [{ "type": "text", "text": "ab" }] },
                    { "type": "paragraph", "content": [{ "type": "text", "text": "cd" }] }
                ]
            }]
        }]
    });
    let document = from_prosemirror_json(&nested, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&document, &schema);
    let first = rendered.find("ab").unwrap();
    let second = rendered.find("cd").unwrap();
    let from = u32::try_from(rendered[..first].chars().count() + 1).unwrap();
    let to = u32::try_from(rendered[..second].chars().count() + 1).unwrap();
    let (actual, expected, _, _, _) = compile_and_execute(
        nested,
        vec![TypedOperation::ReplaceRange {
            range: range_for_test(from, to),
            content: Fragment::from(vec![
                Node::void("hardBreak".into(), HashMap::new()),
                Node::text("Y".into(), vec![]),
            ]),
        }],
    );
    assert_eq!(actual, expected);
    let item = &actual["content"][0]["content"][0]["content"];
    assert_eq!(item.as_array().unwrap().len(), 1);
    assert_eq!(item[0]["content"][0]["text"], "a");
    assert_eq!(item[0]["content"][1]["type"], "hardBreak");
    assert_eq!(item[0]["content"][2]["text"], "Yd");
}

#[test]
fn cross_parent_delete_folds_right_edits_and_accepts_a_survivor_edit() {
    let source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "ab" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "cd" }] }
        ]
    });
    let schema = tiptap_schema();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&document, &schema);
    let first = rendered.find("ab").unwrap();
    let second = rendered.find("cd").unwrap();
    let left = u32::try_from(rendered[..first].chars().count() + 1).unwrap();
    let right = u32::try_from(rendered[..second].chars().count() + 1).unwrap();
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![
            TypedOperation::InsertText {
                at: point_for_test(right),
                text: "X".into(),
                marks: vec![],
            },
            TypedOperation::DeleteRange {
                range: range_for_test(left, right),
            },
            TypedOperation::InsertText {
                at: point_for_test(left),
                text: "Z".into(),
                marks: vec![],
            },
        ],
    );
    assert_eq!(actual, expected);
    assert_eq!(actual["content"].as_array().unwrap().len(), 1);
    assert_eq!(actual["content"][0]["content"][0]["text"], "aZd");
}

#[test]
fn structural_endpoint_resolution_uses_virtual_runs_after_prior_text_edits() {
    let source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "abcd" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "wxyz" }] }
        ]
    });
    let schema = tiptap_schema();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&document, &schema);
    let first = u32::try_from(rendered[..rendered.find("abcd").unwrap()].chars().count()).unwrap();
    let second = u32::try_from(rendered[..rendered.find("wxyz").unwrap()].chars().count()).unwrap();
    let cross = || TypedOperation::DeleteRange {
        range: range_for_test(first + 2, second + 2),
    };
    let bold = || Mark::new("bold".into(), HashMap::new());
    let cases = vec![
        vec![
            TypedOperation::InsertText {
                at: point_for_test(first + 1),
                text: "L".into(),
                marks: vec![],
            },
            cross(),
        ],
        vec![
            TypedOperation::InsertText {
                at: point_for_test(second + 1),
                text: "R".into(),
                marks: vec![],
            },
            cross(),
        ],
        vec![
            TypedOperation::DeleteRange {
                range: range_for_test(first, first + 1),
            },
            cross(),
        ],
        vec![
            TypedOperation::DeleteRange {
                range: range_for_test(second + 2, second + 3),
            },
            cross(),
        ],
        vec![
            TypedOperation::AddMark {
                range: range_for_test(first, first + 1),
                mark: bold(),
            },
            cross(),
        ],
        vec![
            TypedOperation::AddMark {
                range: range_for_test(second + 2, second + 3),
                mark: bold(),
            },
            cross(),
        ],
    ];
    for operations in cases {
        let (actual, expected, _, _, _) = compile_and_execute(source.clone(), operations);
        assert_eq!(actual, expected);
        assert_eq!(actual["content"].as_array().unwrap().len(), 1);
    }
}

#[test]
fn cross_parent_delete_uses_a_nested_list_item_lca() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [{
                "type": "listItem",
                "content": [
                    { "type": "paragraph", "content": [{ "type": "text", "text": "ab" }] },
                    { "type": "paragraph", "content": [{ "type": "text", "text": "cd" }] }
                ]
            }]
        }]
    });
    let schema = tiptap_schema();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&document, &schema);
    let first = rendered.find("ab").unwrap();
    let second = rendered.find("cd").unwrap();
    let from = u32::try_from(rendered[..first].chars().count() + 1).unwrap();
    let to = u32::try_from(rendered[..second].chars().count() + 1).unwrap();
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![TypedOperation::DeleteRange {
            range: range_for_test(from, to),
        }],
    );
    assert_eq!(actual, expected);
    let item_content = actual["content"][0]["content"][0]["content"]
        .as_array()
        .unwrap();
    assert_eq!(item_content.len(), 1);
    assert_eq!(item_content[0]["content"][0]["text"], "ad");
}

#[test]
fn wrap_in_list_replaces_one_complete_root_block_directly() {
    let source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "one" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }
        ]
    });
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![TypedOperation::WrapInList {
            range: range_for_test(0, 3),
            list_type: "bulletList".into(),
            item_type: "listItem".into(),
            attrs: HashMap::new(),
            item_attrs: HashMap::new(),
        }],
    );
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][0]["type"], "bulletList");
    assert_eq!(
        actual["content"][0]["content"][0]["content"][0]["content"][0]["text"],
        "one"
    );
    assert_eq!(actual["content"][1]["content"][0]["text"], "tail");
}

#[test]
fn wrap_in_list_canonicalizes_partial_duplicate_selection_with_typed_attrs() {
    let source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "left" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "same" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "same" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }
        ]
    });
    let schema = attribute_schema();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&document, &schema);
    let occurrences = rendered
        .match_indices("same")
        .map(|(byte, _)| byte)
        .collect::<Vec<_>>();
    let from = u32::try_from(rendered[..occurrences[0]].chars().count() + 1).unwrap();
    let to = u32::try_from(rendered[..occurrences[1]].chars().count() + 2).unwrap();
    let attrs = HashMap::from([("listMeta".into(), json!({ "nested": [1, { "ok": true }] }))]);
    let item_attrs = HashMap::from([
        ("checked".into(), Value::Bool(true)),
        ("itemMeta".into(), json!({ "ids": [1, 2, 3] })),
    ]);
    let operations = || {
        vec![TypedOperation::WrapInList {
            range: range_for_test(from, to),
            list_type: "taskList".into(),
            item_type: "taskItem".into(),
            attrs: attrs.clone(),
            item_attrs: item_attrs.clone(),
        }]
    };
    let (doc, schema, limits, mut compiled) =
        compile_operations_with_schema(&source, operations(), schema);
    assert!(matches!(
        compiled.mutation_plan.actions.as_slice(),
        [
            YrsMutationAction::DeleteXmlChildren {
                child_index: 1,
                child_count: 2,
                operation_index: 0,
                ..
            },
            YrsMutationAction::InsertXmlChildren {
                child_index: 1,
                nodes,
                operation_index: 0,
                ..
            }
        ] if nodes.len() == 1 && nodes[0].index == 1
    ));
    let (
        left_block_id,
        left_text_id,
        moved_block_ids,
        tail_block_id,
        tail_text_id,
        tail_sticky,
        before_full_len,
        before_update,
    ) = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let children = fragment.children(&txn).collect::<Vec<_>>();
        let left_text = paragraph_text(&fragment, &txn, 0);
        let tail_text = paragraph_text(&fragment, &txn, 3);
        let tail_sticky = StickyIndex::at(
            &txn,
            BranchPtr::from(<XmlTextRef as AsRef<Branch>>::as_ref(&tail_text)),
            2,
            Assoc::After,
        )
        .unwrap();
        (
            children[0].id(),
            <XmlTextRef as AsRef<Branch>>::as_ref(&left_text).id(),
            vec![children[1].id(), children[2].id()],
            children[3].id(),
            <XmlTextRef as AsRef<Branch>>::as_ref(&tail_text).id(),
            tail_sticky,
            txn.encode_state_as_update_v1(&StateVector::default()).len(),
            txn.encode_state_as_update_v1(&StateVector::default()),
        )
    };
    {
        let txn = doc.transact();
        let preflight =
            preflight_mutation_work_for_test(122, &compiled.mutation_plan, &txn).unwrap();
        let exact = compiled.mutation_plan.compilation_work_for_test() + preflight;
        compiled.mutation_plan.set_work_limit_for_test(exact);
        preflight_mutation_plan(122, &compiled.mutation_plan, &txn).unwrap();
        compiled.mutation_plan.set_work_limit_for_test(exact - 1);
        assert_eq!(
            preflight_mutation_plan(122, &compiled.mutation_plan, &txn)
                .unwrap_err()
                .code,
            "OPERATION_LIMIT_EXCEEDED"
        );
        compiled.mutation_plan.set_work_limit_for_test(exact);
    }
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    let estimate = compiled.encoded_growth_bound;
    let undo_exact = compiled.undo_units_bound;
    let update = {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
        txn.commit();
        txn.encode_update_v1()
    };
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let actual = YrsDocumentCodec::new(&schema, &limits)
        .read_json(&fragment, &txn)
        .unwrap();
    assert_eq!(actual, expected);
    assert_eq!(actual["content"].as_array().unwrap().len(), 3);
    let list = &actual["content"][1];
    assert_eq!(list["type"], "taskList");
    assert_eq!(list["attrs"]["listMeta"], attrs["listMeta"]);
    assert_eq!(list["content"].as_array().unwrap().len(), 2);
    for item in list["content"].as_array().unwrap() {
        assert_eq!(item["attrs"]["checked"], true);
        assert_eq!(item["attrs"]["itemMeta"], item_attrs["itemMeta"]);
    }
    let root_children = fragment.children(&txn).collect::<Vec<_>>();
    assert_eq!(root_children[0].id(), left_block_id);
    assert_eq!(root_children[2].id(), tail_block_id);
    assert!(moved_block_ids
        .iter()
        .all(|id| root_children.iter().all(|child| child.id() != *id)));
    assert_eq!(
        <XmlTextRef as AsRef<Branch>>::as_ref(&paragraph_text(&fragment, &txn, 0)).id(),
        left_text_id
    );
    assert_eq!(
        <XmlTextRef as AsRef<Branch>>::as_ref(&paragraph_text(&fragment, &txn, 2)).id(),
        tail_text_id
    );
    let resolved_sticky = tail_sticky.get_offset(&txn).unwrap();
    assert_eq!(resolved_sticky.branch.id(), tail_text_id);
    assert_eq!(resolved_sticky.index, 2);
    assert!(update.len() <= estimate, "{} > {estimate}", update.len());
    let after_full_len = txn.encode_state_as_update_v1(&StateVector::default()).len();
    assert!(after_full_len <= before_full_len + estimate);
    let replica = utf16_doc();
    {
        let mut replica_txn = replica.transact_mut();
        replica_txn
            .apply_update(Update::decode_v1(&before_update).unwrap())
            .unwrap();
        replica_txn
            .apply_update(Update::decode_v1(&update).unwrap())
            .unwrap();
    }
    let replica_txn = replica.transact();
    let replica_fragment = replica_txn.get_xml_fragment("prosemirror").unwrap();
    assert_eq!(
        YrsDocumentCodec::new(&schema, &limits)
            .read_json(&replica_fragment, &replica_txn)
            .unwrap(),
        expected
    );
    assert_eq!(
        compile_operations_with_undo_limit(&source, operations(), attribute_schema(), undo_exact)
            .unwrap()
            .undo_units_bound,
        undo_exact
    );
    let undo_error = compile_operations_with_undo_limit(
        &source,
        operations(),
        attribute_schema(),
        undo_exact - 1,
    )
    .unwrap_err();
    assert_eq!(undo_error.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(undo_error.actual, Some(undo_exact));
}

#[test]
fn wrap_in_list_handles_void_and_empty_blocks_without_existing_text_targets() {
    for source in [
        json!({
            "type": "doc",
            "content": [
                { "type": "paragraph", "content": [{ "type": "hardBreak" }] },
                { "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }
            ]
        }),
        json!({
            "type": "doc",
            "content": [
                { "type": "paragraph" },
                { "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }
            ]
        }),
    ] {
        let (actual, expected, _, _, _) = compile_and_execute(
            source,
            vec![TypedOperation::WrapInList {
                range: range_for_test(0, 1),
                list_type: "bulletList".into(),
                item_type: "listItem".into(),
                attrs: HashMap::new(),
                item_attrs: HashMap::new(),
            }],
        );
        assert_eq!(actual, expected);
        assert_eq!(actual["content"][0]["type"], "bulletList");
        assert_eq!(actual["content"][1]["content"][0]["text"], "tail");
    }
}

#[test]
fn wrap_empty_textblock_accepts_follow_up_text_in_prepared_blueprint() {
    let source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph" },
            { "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }
        ]
    });
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![
            TypedOperation::WrapInList {
                range: range_for_test(0, 1),
                list_type: "bulletList".into(),
                item_type: "listItem".into(),
                attrs: HashMap::new(),
                item_attrs: HashMap::new(),
            },
            TypedOperation::InsertText {
                at: point_for_test(1),
                text: "filled".into(),
                marks: vec![],
            },
        ],
    );
    assert_eq!(actual, expected);
    assert_eq!(
        actual["content"][0]["content"][0]["content"][0]["content"][0]["text"],
        "filled"
    );
}

#[test]
fn wrap_then_unwrap_same_transaction_rewrites_owned_blueprint() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "one" }]
        }]
    });
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::WrapInList {
                range: range_for_test(0, 3),
                list_type: "bulletList".into(),
                item_type: "listItem".into(),
                attrs: HashMap::new(),
                item_attrs: HashMap::new(),
            },
            TypedOperation::UnwrapFromList {
                at: point_for_test(1),
            },
        ],
        tiptap_schema(),
    );
    assert!(
        compiled.mutation_plan.actions.is_empty(),
        "wrapping and immediately unwrapping the same unchanged block must cancel"
    );
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let actual = YrsDocumentCodec::new(&schema, &limits)
        .read_json(&fragment, &txn)
        .unwrap();
    assert_eq!(actual, source);
    assert_eq!(actual, expected);
}

#[test]
fn wrap_edit_then_unwrap_rewrites_the_owned_blueprint_once() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "one" }]
        }]
    });
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::WrapInList {
                range: range_for_test(0, 3),
                list_type: "bulletList".into(),
                item_type: "listItem".into(),
                attrs: HashMap::new(),
                item_attrs: HashMap::new(),
            },
            TypedOperation::InsertText {
                at: point_for_test(1),
                text: "X".into(),
                marks: vec![],
            },
            TypedOperation::UnwrapFromList {
                at: point_for_test(1),
            },
        ],
        tiptap_schema(),
    );
    assert_eq!(
        compiled
            .mutation_plan
            .actions
            .iter()
            .filter(|action| matches!(action, YrsMutationAction::DeleteXmlChildren { .. }))
            .count(),
        1
    );
    assert_eq!(
        compiled
            .mutation_plan
            .actions
            .iter()
            .filter(|action| matches!(action, YrsMutationAction::InsertXmlChildren { .. }))
            .count(),
        1
    );
    assert!(!compiled
        .mutation_plan
        .actions
        .iter()
        .any(|action| matches!(action, YrsMutationAction::InsertText { .. })));
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let actual = YrsDocumentCodec::new(&schema, &limits)
        .read_json(&fragment, &txn)
        .unwrap();
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][0]["content"][0]["text"], "oXne");
}

#[test]
fn wrap_in_list_folds_prior_edits_and_accepts_follow_up_prepared_edits() {
    let source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "ab" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }
        ]
    });
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::InsertText {
                at: point_for_test(1),
                text: "X".into(),
                marks: vec![],
            },
            TypedOperation::WrapInList {
                range: range_for_test(0, 2),
                list_type: "bulletList".into(),
                item_type: "listItem".into(),
                attrs: HashMap::new(),
                item_attrs: HashMap::new(),
            },
            TypedOperation::InsertText {
                at: point_for_test(1),
                text: "Y".into(),
                marks: vec![],
            },
        ],
        tiptap_schema(),
    );
    assert!(!compiled
        .mutation_plan
        .actions
        .iter()
        .any(|action| matches!(action, YrsMutationAction::InsertText { .. })));
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let actual = YrsDocumentCodec::new(&schema, &limits)
        .read_json(&fragment, &txn)
        .unwrap();
    assert_eq!(actual, expected);
    let text = actual["content"][0]["content"][0]["content"][0]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert!(text.contains('X') && text.contains('Y'));
}

#[test]
fn structural_insert_then_wrap_folds_blueprint_and_accepts_follow_up_text() {
    let source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "ab" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }
        ]
    });
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::InsertNode {
                at: point_for_test(1),
                node: Node::void("hardBreak".into(), HashMap::new()),
            },
            TypedOperation::WrapInList {
                range: range_for_test(0, 2),
                list_type: "bulletList".into(),
                item_type: "listItem".into(),
                attrs: HashMap::new(),
                item_attrs: HashMap::new(),
            },
            TypedOperation::InsertText {
                at: point_for_test(1),
                text: "Z".into(),
                marks: vec![],
            },
        ],
        tiptap_schema(),
    );
    assert_eq!(compiled.mutation_plan.actions.len(), 2);
    assert!(matches!(
        compiled.mutation_plan.actions[0],
        YrsMutationAction::DeleteXmlChildren { .. }
    ));
    assert!(matches!(
        compiled.mutation_plan.actions[1],
        YrsMutationAction::InsertXmlChildren { .. }
    ));
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let actual = YrsDocumentCodec::new(&schema, &limits)
        .read_json(&fragment, &txn)
        .unwrap();
    assert_eq!(actual, expected);
    let paragraph_content = actual["content"][0]["content"][0]["content"][0]["content"]
        .as_array()
        .unwrap();
    assert!(paragraph_content
        .iter()
        .any(|node| node["type"] == "hardBreak"));
    assert!(paragraph_content
        .iter()
        .any(|node| node["text"].as_str().is_some_and(|text| text.contains('Z'))));
}

#[test]
fn unwrap_only_list_item_replaces_the_list_with_its_blocks_directly() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [{
                "type": "listItem",
                "content": [{
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": "one" }]
                }]
            }]
        }]
    });
    let schema = tiptap_schema();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&document, &schema);
    let one = u32::try_from(rendered[..rendered.find("one").unwrap()].chars().count()).unwrap();
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![TypedOperation::UnwrapFromList {
            at: point_for_test(one + 1),
        }],
    );
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][0]["type"], "paragraph");
    assert_eq!(actual["content"][0]["content"][0]["text"], "one");
}

#[test]
fn unwrap_only_then_insert_text_folds_into_prepared_text() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [{
                "type": "listItem",
                "content": [{
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": "one" }]
                }]
            }]
        }]
    });
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::UnwrapFromList {
                at: point_for_test(3),
            },
            TypedOperation::InsertText {
                at: point_for_test(3),
                text: "X".into(),
                marks: vec![],
            },
        ],
        tiptap_schema(),
    );
    assert!(!compiled
        .mutation_plan
        .actions
        .iter()
        .any(|action| matches!(action, YrsMutationAction::InsertText { .. })));
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let actual = YrsDocumentCodec::new(&schema, &limits)
        .read_json(&fragment, &txn)
        .unwrap();
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][0]["content"][0]["text"], "oXne");
}

#[test]
fn edit_then_unwrap_tombstones_deleted_text_actions() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [{
                "type": "listItem",
                "content": [{
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": "one" }]
                }]
            }]
        }]
    });
    let schema = tiptap_schema();
    let one = rendered_scalar_offset(&source, &schema, "one");
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::InsertText {
                at: point_for_test(one + 1),
                text: "X".into(),
                marks: vec![],
            },
            TypedOperation::UnwrapFromList {
                at: point_for_test(one + 1),
            },
        ],
        schema,
    );
    assert!(!compiled
        .mutation_plan
        .actions
        .iter()
        .any(|action| matches!(action, YrsMutationAction::InsertText { .. })));
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let actual = YrsDocumentCodec::new(&schema, &limits)
        .read_json(&fragment, &txn)
        .unwrap();
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][0]["content"][0]["text"], "oXne");
}

#[test]
fn insert_node_then_unwrap_owns_one_canonical_prepared_batch() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [{
                "type": "listItem",
                "content": [{
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": "one" }]
                }]
            }]
        }]
    });
    let schema = tiptap_schema();
    let one = rendered_scalar_offset(&source, &schema, "one");
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::InsertNode {
                at: point_for_test(one + 1),
                node: Node::void("hardBreak".into(), HashMap::new()),
            },
            TypedOperation::UnwrapFromList {
                at: point_for_test(one + 1),
            },
        ],
        schema,
    );
    assert!(matches!(
        compiled.mutation_plan.actions.as_slice(),
        [
            YrsMutationAction::DeleteXmlChildren {
                child_index: 0,
                child_count: 1,
                ..
            },
            YrsMutationAction::InsertXmlChildren { nodes, .. }
        ] if nodes.len() == 1
    ));
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let actual = YrsDocumentCodec::new(&schema, &limits)
        .read_json(&fragment, &txn)
        .unwrap();
    assert_eq!(actual, expected);
    assert!(actual["content"][0]["content"]
        .as_array()
        .unwrap()
        .iter()
        .any(|node| node["type"] == "hardBreak"));
}

#[test]
fn unwrap_only_then_update_extracted_block_attrs() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "taskList",
            "content": [{
                "type": "taskItem",
                "content": [
                    {
                        "type": "paragraph",
                        "content": [{ "type": "text", "text": "lead" }]
                    },
                    {
                        "type": "h2",
                        "attrs": { "id": "before" },
                        "content": [{ "type": "text", "text": "heading" }]
                    }
                ]
            }]
        }]
    });
    let schema = attribute_schema();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&document, &schema);
    let lead = u32::try_from(rendered[..rendered.find("lead").unwrap()].chars().count()).unwrap();
    let heading = u32::try_from(
        rendered[..rendered.find("heading").unwrap()]
            .chars()
            .count(),
    )
    .unwrap();
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::UnwrapFromList {
                at: point_for_test(lead + 1),
            },
            TypedOperation::UpdateNodeAttrs {
                at: point_for_test(heading),
                attrs: HashMap::from([("id".into(), Value::String("after".into()))]),
            },
        ],
        schema,
    );
    assert!(!compiled.mutation_plan.actions.iter().any(|action| {
        matches!(
            action,
            YrsMutationAction::SetXmlAttribute { .. }
                | YrsMutationAction::RemoveXmlAttribute { .. }
        )
    }));
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let actual = YrsDocumentCodec::new(&schema, &limits)
        .read_json(&fragment, &txn)
        .unwrap();
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][1]["attrs"]["id"], "after");
}

#[test]
fn attrs_then_unwrap_tombstones_deleted_element_attrs() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "taskList",
            "content": [{
                "type": "taskItem",
                "content": [
                    {
                        "type": "paragraph",
                        "content": [{ "type": "text", "text": "lead" }]
                    },
                    {
                        "type": "h2",
                        "attrs": { "id": "before" },
                        "content": [{ "type": "text", "text": "heading" }]
                    }
                ]
            }]
        }]
    });
    let schema = attribute_schema();
    let lead = rendered_scalar_offset(&source, &schema, "lead");
    let heading = rendered_scalar_offset(&source, &schema, "heading");
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::UpdateNodeAttrs {
                at: point_for_test(heading),
                attrs: HashMap::from([("id".into(), Value::String("after".into()))]),
            },
            TypedOperation::UnwrapFromList {
                at: point_for_test(lead + 1),
            },
        ],
        schema,
    );
    assert!(!compiled.mutation_plan.actions.iter().any(|action| {
        matches!(
            action,
            YrsMutationAction::SetXmlAttribute { .. }
                | YrsMutationAction::RemoveXmlAttribute { .. }
        )
    }));
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let actual = YrsDocumentCodec::new(&schema, &limits)
        .read_json(&fragment, &txn)
        .unwrap();
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][1]["attrs"]["id"], "after");
}

#[test]
fn indent_first_list_item_is_an_exact_compiler_noop() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }] },
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }] }
            ]
        }]
    });
    let schema = tiptap_schema();
    let one = rendered_scalar_offset(&source, &schema, "one") + 1;
    let (_, _, _, compiled) = compile_operations_with_schema(
        &source,
        vec![TypedOperation::IndentListItem {
            at: point_for_test(one),
        }],
        schema,
    );
    assert_eq!(
        to_prosemirror_json(&compiled.preview, &tiptap_schema()),
        source
    );
    assert!(compiled.mutation_plan.actions.is_empty());
    assert_eq!(compiled.encoded_growth_bound, 0);
    assert_eq!(compiled.undo_units_bound, 0);
}

#[test]
fn indent_list_item_creates_a_direct_nested_list_and_matches_replica() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "orderedList",
            "attrs": { "start": 3 },
            "content": [
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }] },
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }] },
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "three" }] }] }
            ]
        }]
    });
    let schema = tiptap_schema();
    let two = rendered_scalar_offset(&source, &schema, "two") + 1;
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![TypedOperation::IndentListItem {
            at: point_for_test(two),
        }],
    );
    assert_eq!(actual, expected);
    let outer = &actual["content"][0];
    assert_eq!(outer["content"].as_array().unwrap().len(), 2);
    let nested = &outer["content"][0]["content"][1];
    assert_eq!(nested["type"], "orderedList");
    assert_eq!(nested["attrs"]["start"], 3);
    assert_eq!(
        nested["content"][0]["content"][0]["content"][0]["text"],
        "two"
    );
    assert_eq!(
        outer["content"][1]["content"][0]["content"][0]["text"],
        "three"
    );
}

#[test]
fn indent_appends_to_existing_final_same_type_list_and_preserves_stationary_ids() {
    let source = json!({
        "type": "doc",
        "content": [
            {
                "type": "bulletList",
                "content": [
                    {
                        "type": "listItem",
                        "content": [
                            { "type": "paragraph", "content": [{ "type": "text", "text": "one" }] },
                            {
                                "type": "bulletList",
                                "content": [{
                                    "type": "listItem",
                                    "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "nested" }] }]
                                }]
                            }
                        ]
                    },
                    { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }] },
                    { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "three" }] }] }
                ]
            },
            { "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }
        ]
    });
    let schema = tiptap_schema();
    let two = rendered_scalar_offset(&source, &schema, "two") + 1;
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![TypedOperation::IndentListItem {
            at: point_for_test(two),
        }],
        schema,
    );
    let (outer_id, first_id, nested_id, nested_item_id, tail_item_id, nested_sticky) = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let XmlOut::Element(outer) = fragment.get(&txn, 0).unwrap() else {
            panic!("outer list expected")
        };
        let items = outer.children(&txn).collect::<Vec<_>>();
        let XmlOut::Element(first) = &items[0] else {
            panic!("first item expected")
        };
        let XmlOut::Element(nested) = first.get(&txn, 1).unwrap() else {
            panic!("nested list expected")
        };
        let nested_item = nested.get(&txn, 0).unwrap();
        let nested_text = list_item_text(&nested_item, &txn);
        (
            AsRef::<Branch>::as_ref(&outer).id(),
            items[0].id(),
            AsRef::<Branch>::as_ref(&nested).id(),
            nested_item.id(),
            items[2].id(),
            StickyIndex::at(
                &txn,
                BranchPtr::from(<XmlTextRef as AsRef<Branch>>::as_ref(&nested_text)),
                2,
                Assoc::After,
            )
            .unwrap(),
        )
    };
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let XmlOut::Element(outer) = fragment.get(&txn, 0).unwrap() else {
        panic!("outer list expected")
    };
    let items = outer.children(&txn).collect::<Vec<_>>();
    let XmlOut::Element(first) = &items[0] else {
        panic!("first item expected")
    };
    let XmlOut::Element(nested) = first.get(&txn, 1).unwrap() else {
        panic!("nested list expected")
    };
    assert_eq!(AsRef::<Branch>::as_ref(&outer).id(), outer_id);
    assert_eq!(items[0].id(), first_id);
    assert_eq!(AsRef::<Branch>::as_ref(&nested).id(), nested_id);
    assert_eq!(nested.get(&txn, 0).unwrap().id(), nested_item_id);
    assert_eq!(items[1].id(), tail_item_id);
    assert_eq!(nested_sticky.get_offset(&txn).unwrap().index, 2);
    assert_eq!(
        YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap(),
        expected
    );
}

#[test]
fn indent_respects_different_and_nonfinal_nested_lists() {
    for previous_tail in [
        json!({
            "type": "bulletList",
            "content": [{
                "type": "listItem",
                "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "different" }] }]
            }]
        }),
        json!({
            "type": "blockquote",
            "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "after-list" }] }]
        }),
    ] {
        let mut previous_content =
            vec![json!({ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] })];
        if previous_tail["type"] == "blockquote" {
            previous_content.push(json!({
                "type": "orderedList",
                "content": [{
                    "type": "listItem",
                    "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "existing" }] }]
                }]
            }));
        }
        previous_content.push(previous_tail);
        let source = json!({
            "type": "doc",
            "content": [{
                "type": "orderedList",
                "attrs": { "start": 4 },
                "content": [
                    { "type": "listItem", "content": previous_content },
                    { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }] }
                ]
            }]
        });
        let schema = tiptap_schema();
        let two = rendered_scalar_offset(&source, &schema, "two") + 1;
        let (actual, expected, _, _, _) = compile_and_execute(
            source,
            vec![TypedOperation::IndentListItem {
                at: point_for_test(two),
            }],
        );
        assert_eq!(actual, expected);
        let children = actual["content"][0]["content"][0]["content"]
            .as_array()
            .unwrap();
        let appended = children.last().unwrap();
        assert_eq!(appended["type"], "orderedList");
        assert_eq!(appended["attrs"]["start"], 4);
        assert_eq!(
            appended["content"][0]["content"][0]["content"][0]["text"],
            "two"
        );
    }
}

#[test]
fn indent_is_role_driven_for_task_items_and_materializes_empty_textblocks() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "taskList",
            "attrs": { "listMeta": { "owner": "team" } },
            "content": [
                {
                    "type": "taskItem",
                    "attrs": { "checked": false, "itemMeta": { "id": "one" } },
                    "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }]
                },
                {
                    "type": "taskItem",
                    "attrs": { "checked": true, "itemMeta": { "id": "empty" } },
                    "content": [{ "type": "paragraph" }]
                }
            ]
        }]
    });
    let schema = attribute_schema();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let at = u32::try_from(
        crate::render::rendered_text(&document, &schema)
            .chars()
            .count(),
    )
    .unwrap();
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![TypedOperation::IndentListItem {
            at: point_for_test(at),
        }],
        schema,
    );
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let actual = YrsDocumentCodec::new(&schema, &limits)
        .read_json(&fragment, &txn)
        .unwrap();
    assert_eq!(actual, expected);
    let nested = &actual["content"][0]["content"][0]["content"][1];
    assert_eq!(nested["type"], "taskList");
    assert_eq!(nested["attrs"]["listMeta"]["owner"], "team");
    assert_eq!(nested["content"][0]["type"], "taskItem");
    assert_eq!(nested["content"][0]["attrs"]["checked"], true);
    assert_eq!(nested["content"][0]["attrs"]["itemMeta"]["id"], "empty");
    assert_eq!(nested["content"][0]["content"][0]["type"], "paragraph");
}

#[test]
fn indent_folds_prior_and_follow_up_edits_into_the_moved_prepared_item() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }] },
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }] }
            ]
        }]
    });
    let schema = tiptap_schema();
    let two = rendered_scalar_offset(&source, &schema, "two") + 1;
    for operations in [
        vec![
            TypedOperation::InsertText {
                at: point_for_test(two),
                text: "X".into(),
                marks: vec![],
            },
            TypedOperation::IndentListItem {
                at: point_for_test(two),
            },
        ],
        vec![
            TypedOperation::IndentListItem {
                at: point_for_test(two),
            },
            TypedOperation::InsertText {
                at: point_for_test(two),
                text: "X".into(),
                marks: vec![],
            },
        ],
    ] {
        let (doc, schema, limits, compiled) =
            compile_operations_with_schema(&source, operations, tiptap_schema());
        assert!(!compiled
            .mutation_plan
            .actions
            .iter()
            .any(|action| matches!(action, YrsMutationAction::InsertText { .. })));
        let expected = to_prosemirror_json(&compiled.preview, &schema);
        {
            let mut txn = doc.transact_mut();
            execute_mutation_plan(compiled.mutation_plan, &mut txn);
        }
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let actual = YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap();
        assert_eq!(actual, expected);
        assert_eq!(
            actual["content"][0]["content"][0]["content"][1]["content"][0]["content"][0]["content"]
                [0]["text"],
            "tXwo"
        );
    }
}

#[test]
fn wrap_then_indent_rewrites_the_single_owned_prepared_batch() {
    let source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "one" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }
        ]
    });
    let schema = tiptap_schema();
    let two = rendered_scalar_offset(&source, &schema, "two");
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::WrapInList {
                range: range_for_test(0, two + 3),
                list_type: "bulletList".into(),
                item_type: "listItem".into(),
                attrs: HashMap::new(),
                item_attrs: HashMap::new(),
            },
            TypedOperation::IndentListItem {
                at: point_for_test(two + 1),
            },
        ],
        schema,
    );
    assert_eq!(
        compiled
            .mutation_plan
            .actions
            .iter()
            .filter(|action| matches!(action, YrsMutationAction::InsertXmlChildren { .. }))
            .count(),
        1
    );
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    assert_eq!(
        YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap(),
        expected
    );
}

#[test]
fn repeated_indent_appends_into_the_prepared_existing_nested_list() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }] },
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }] },
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "three" }] }] }
            ]
        }]
    });
    let schema = tiptap_schema();
    let two = rendered_scalar_offset(&source, &schema, "two") + 1;
    let three = rendered_scalar_offset(&source, &schema, "three") + 1;
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![
            TypedOperation::IndentListItem {
                at: point_for_test(two),
            },
            TypedOperation::IndentListItem {
                at: point_for_test(three),
            },
        ],
    );
    assert_eq!(actual, expected);
    let nested_items = actual["content"][0]["content"][0]["content"][1]["content"]
        .as_array()
        .unwrap();
    assert_eq!(nested_items.len(), 2);
    assert_eq!(nested_items[0]["content"][0]["content"][0]["text"], "two");
    assert_eq!(nested_items[1]["content"][0]["content"][0]["text"], "three");
}

#[test]
fn indent_then_update_attrs_targets_the_moved_prepared_task_item() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "taskList",
            "content": [
                {
                    "type": "taskItem",
                    "attrs": { "checked": false, "itemMeta": { "id": "one" } },
                    "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }]
                },
                {
                    "type": "taskItem",
                    "attrs": { "checked": true, "itemMeta": { "id": "two" } },
                    "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }]
                }
            ]
        }]
    });
    let schema = attribute_schema();
    let two = rendered_scalar_offset(&source, &schema, "two") + 1;
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::IndentListItem {
                at: point_for_test(two),
            },
            TypedOperation::UpdateNodeAttrs {
                at: point_for_test(two),
                attrs: HashMap::from([
                    ("checked".into(), Value::Bool(false)),
                    ("itemMeta".into(), json!({ "id": "updated" })),
                ]),
            },
        ],
        schema,
    );
    assert!(!compiled.mutation_plan.actions.iter().any(|action| {
        matches!(
            action,
            YrsMutationAction::SetXmlAttribute { .. }
                | YrsMutationAction::RemoveXmlAttribute { .. }
        )
    }));
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let actual = YrsDocumentCodec::new(&schema, &limits)
        .read_json(&fragment, &txn)
        .unwrap();
    assert_eq!(actual, expected);
    let moved = &actual["content"][0]["content"][0]["content"][1]["content"][0];
    assert_eq!(moved["attrs"]["checked"], false);
    assert_eq!(moved["attrs"]["itemMeta"]["id"], "updated");
}

#[test]
fn indent_preflight_growth_undo_and_replica_bounds_are_exactly_enforced() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }] },
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }] },
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "three" }] }] }
            ]
        }]
    });
    let schema = tiptap_schema();
    let two = rendered_scalar_offset(&source, &schema, "two") + 1;
    let operations = || {
        vec![TypedOperation::IndentListItem {
            at: point_for_test(two),
        }]
    };
    let (doc, schema, limits, mut compiled) =
        compile_operations_with_schema(&source, operations(), schema);
    let (before_update, before_full_len) = {
        let txn = doc.transact();
        let update = txn.encode_state_as_update_v1(&StateVector::default());
        let len = update.len();
        (update, len)
    };
    {
        let txn = doc.transact();
        let preflight =
            preflight_mutation_work_for_test(122, &compiled.mutation_plan, &txn).unwrap();
        let exact = compiled.mutation_plan.compilation_work_for_test() + preflight;
        compiled.mutation_plan.set_work_limit_for_test(exact);
        preflight_mutation_plan(122, &compiled.mutation_plan, &txn).unwrap();
        compiled.mutation_plan.set_work_limit_for_test(exact - 1);
        let error = preflight_mutation_plan(122, &compiled.mutation_plan, &txn).unwrap_err();
        assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED");
        assert_eq!(error.limit, Some(u64::try_from(exact - 1).unwrap()));
        assert!(error
            .actual
            .is_some_and(|actual| actual > u64::try_from(exact - 1).unwrap()));
        compiled.mutation_plan.set_work_limit_for_test(exact);
    }
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    let growth_bound = compiled.encoded_growth_bound;
    let undo_bound = compiled.undo_units_bound;
    assert!(growth_bound > 0);
    assert!(undo_bound > 0);
    assert_eq!(
        compile_operations_with_undo_limit(&source, operations(), tiptap_schema(), undo_bound)
            .unwrap()
            .undo_units_bound,
        undo_bound
    );
    let undo_error =
        compile_operations_with_undo_limit(&source, operations(), tiptap_schema(), undo_bound - 1)
            .unwrap_err();
    assert_eq!(undo_error.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(undo_error.actual, Some(undo_bound));

    let fragment = doc.transact().get_xml_fragment("prosemirror").unwrap();
    let mut undo = UndoManager::<()>::new();
    undo.expand_scope(&doc, &fragment);
    let update = {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
        txn.commit();
        txn.encode_update_v1()
    };
    assert!(update.len() <= growth_bound);
    {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        assert_eq!(
            YrsDocumentCodec::new(&schema, &limits)
                .read_json(&fragment, &txn)
                .unwrap(),
            expected
        );
        assert!(
            txn.encode_state_as_update_v1(&StateVector::default()).len()
                <= before_full_len + growth_bound
        );
    }
    let replica = utf16_doc();
    {
        let mut txn = replica.transact_mut();
        txn.apply_update(Update::decode_v1(&before_update).unwrap())
            .unwrap();
        txn.apply_update(Update::decode_v1(&update).unwrap())
            .unwrap();
    }
    {
        let txn = replica.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        assert_eq!(
            YrsDocumentCodec::new(&schema, &limits)
                .read_json(&fragment, &txn)
                .unwrap(),
            expected
        );
    }
    assert!(undo.undo_blocking());
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    assert_eq!(
        YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap(),
        source
    );
}

#[test]
fn outdent_top_level_list_item_is_an_exact_compiler_noop() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }] },
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }] }
            ]
        }]
    });
    let schema = tiptap_schema();
    let two = rendered_scalar_offset(&source, &schema, "two") + 1;
    let (_, _, _, compiled) = compile_operations_with_schema(
        &source,
        vec![TypedOperation::OutdentListItem {
            at: point_for_test(two),
        }],
        schema,
    );
    assert_eq!(
        to_prosemirror_json(&compiled.preview, &tiptap_schema()),
        source
    );
    assert!(compiled.mutation_plan.actions.is_empty());
    assert_eq!(compiled.encoded_growth_bound, 0);
    assert_eq!(compiled.undo_units_bound, 0);
}

#[test]
fn outdent_first_middle_and_last_nested_items_execute_directly() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [
                {
                    "type": "listItem",
                    "content": [
                        { "type": "paragraph", "content": [{ "type": "text", "text": "parent" }] },
                        {
                            "type": "bulletList",
                            "content": [
                                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }] },
                                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }] },
                                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "three" }] }] }
                            ]
                        }
                    ]
                },
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }] }
            ]
        }]
    });
    for (selected, expected_before, expected_after) in
        [("one", 0usize, 2usize), ("two", 1, 1), ("three", 2, 0)]
    {
        let schema = tiptap_schema();
        let at = rendered_scalar_offset(&source, &schema, selected) + 1;
        let (actual, expected, _, _, _) = compile_and_execute(
            source.clone(),
            vec![TypedOperation::OutdentListItem {
                at: point_for_test(at),
            }],
        );
        assert_eq!(actual, expected);
        let outer = actual["content"][0]["content"].as_array().unwrap();
        assert_eq!(outer.len(), 3);
        assert_eq!(outer[1]["content"][0]["content"][0]["text"], selected);
        let parent_content = outer[0]["content"].as_array().unwrap();
        if expected_before == 0 {
            assert_eq!(parent_content.len(), 1);
        } else {
            assert_eq!(
                parent_content[1]["content"].as_array().unwrap().len(),
                expected_before
            );
        }
        let moved_content = outer[1]["content"].as_array().unwrap();
        if expected_after == 0 {
            assert_eq!(moved_content.len(), 1);
        } else {
            assert_eq!(
                moved_content[1]["content"].as_array().unwrap().len(),
                expected_after
            );
        }
        assert_eq!(outer[2]["content"][0]["content"][0]["text"], "tail");
    }
}

#[test]
fn outdent_preserves_existing_final_nested_list_attrs_when_merging_trailing_items() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "orderedList",
            "attrs": { "start": 10 },
            "content": [
                {
                    "type": "listItem",
                    "content": [
                        { "type": "paragraph", "content": [{ "type": "text", "text": "parent" }] },
                        {
                            "type": "orderedList",
                            "attrs": { "start": 5 },
                            "content": [
                                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "before" }] }] },
                                {
                                    "type": "listItem",
                                    "content": [
                                        { "type": "paragraph", "content": [{ "type": "text", "text": "moved" }] },
                                        {
                                            "type": "orderedList",
                                            "attrs": { "start": 99 },
                                            "content": [{ "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "existing" }] }] }]
                                        }
                                    ]
                                },
                                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "after-one" }] }] },
                                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "after-two" }] }] }
                            ]
                        }
                    ]
                },
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }] }
            ]
        }]
    });
    let schema = tiptap_schema();
    let moved = rendered_scalar_offset(&source, &schema, "moved") + 1;
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![TypedOperation::OutdentListItem {
            at: point_for_test(moved),
        }],
    );
    assert_eq!(actual, expected);
    let merged = &actual["content"][0]["content"][1]["content"][1];
    assert_eq!(merged["type"], "orderedList");
    assert_eq!(merged["attrs"]["start"], 99);
    assert_eq!(merged["content"].as_array().unwrap().len(), 3);
    assert_eq!(
        merged["content"][0]["content"][0]["content"][0]["text"],
        "existing"
    );
    assert_eq!(
        merged["content"][1]["content"][0]["content"][0]["text"],
        "after-one"
    );
    assert_eq!(
        merged["content"][2]["content"][0]["content"][0]["text"],
        "after-two"
    );
}

#[test]
fn outdent_preserves_stationary_parent_prefix_tail_ids_and_sticky() {
    let source = json!({
        "type": "doc",
        "content": [
            {
                "type": "bulletList",
                "content": [
                    {
                        "type": "listItem",
                        "content": [
                            { "type": "paragraph", "content": [{ "type": "text", "text": "parent" }] },
                            {
                                "type": "bulletList",
                                "content": [
                                    { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }] },
                                    { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }] },
                                    { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "three" }] }] }
                                ]
                            }
                        ]
                    },
                    { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }] }
                ]
            },
            { "type": "paragraph", "content": [{ "type": "text", "text": "after" }] }
        ]
    });
    let schema = tiptap_schema();
    let two = rendered_scalar_offset(&source, &schema, "two") + 1;
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![TypedOperation::OutdentListItem {
            at: point_for_test(two),
        }],
        schema,
    );
    let (outer_id, parent_id, nested_id, prefix_id, tail_id, after_id, sticky) = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let XmlOut::Element(outer) = fragment.get(&txn, 0).unwrap() else {
            panic!("outer list expected")
        };
        let items = outer.children(&txn).collect::<Vec<_>>();
        let XmlOut::Element(parent) = &items[0] else {
            panic!("parent item expected")
        };
        let XmlOut::Element(nested) = parent.get(&txn, 1).unwrap() else {
            panic!("nested list expected")
        };
        let prefix = nested.get(&txn, 0).unwrap();
        let prefix_text = list_item_text(&prefix, &txn);
        (
            AsRef::<Branch>::as_ref(&outer).id(),
            items[0].id(),
            AsRef::<Branch>::as_ref(&nested).id(),
            prefix.id(),
            items[1].id(),
            fragment.get(&txn, 1).unwrap().id(),
            StickyIndex::at(
                &txn,
                BranchPtr::from(<XmlTextRef as AsRef<Branch>>::as_ref(&prefix_text)),
                1,
                Assoc::After,
            )
            .unwrap(),
        )
    };
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let XmlOut::Element(outer) = fragment.get(&txn, 0).unwrap() else {
        panic!("outer list expected")
    };
    let items = outer.children(&txn).collect::<Vec<_>>();
    let XmlOut::Element(parent) = &items[0] else {
        panic!("parent item expected")
    };
    let XmlOut::Element(nested) = parent.get(&txn, 1).unwrap() else {
        panic!("nested list expected")
    };
    assert_eq!(AsRef::<Branch>::as_ref(&outer).id(), outer_id);
    assert_eq!(items[0].id(), parent_id);
    assert_eq!(AsRef::<Branch>::as_ref(&nested).id(), nested_id);
    assert_eq!(nested.get(&txn, 0).unwrap().id(), prefix_id);
    assert_eq!(items[2].id(), tail_id);
    assert_eq!(fragment.get(&txn, 1).unwrap().id(), after_id);
    assert_eq!(sticky.get_offset(&txn).unwrap().index, 1);
    assert_eq!(
        YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap(),
        expected
    );
}

#[test]
fn outdent_preflight_growth_undo_and_replica_bounds_are_exactly_enforced() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [
                {
                    "type": "listItem",
                    "content": [
                        { "type": "paragraph", "content": [{ "type": "text", "text": "parent" }] },
                        {
                            "type": "bulletList",
                            "content": [
                                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }] },
                                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }] },
                                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "three" }] }] }
                            ]
                        }
                    ]
                },
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }] }
            ]
        }]
    });
    let schema = tiptap_schema();
    let two = rendered_scalar_offset(&source, &schema, "two") + 1;
    let operations = || {
        vec![TypedOperation::OutdentListItem {
            at: point_for_test(two),
        }]
    };
    let (doc, schema, limits, mut compiled) =
        compile_operations_with_schema(&source, operations(), schema);
    let (before_update, before_full_len) = {
        let txn = doc.transact();
        let update = txn.encode_state_as_update_v1(&StateVector::default());
        let len = update.len();
        (update, len)
    };
    {
        let txn = doc.transact();
        let preflight =
            preflight_mutation_work_for_test(122, &compiled.mutation_plan, &txn).unwrap();
        let exact = compiled.mutation_plan.compilation_work_for_test() + preflight;
        compiled.mutation_plan.set_work_limit_for_test(exact);
        preflight_mutation_plan(122, &compiled.mutation_plan, &txn).unwrap();
        compiled.mutation_plan.set_work_limit_for_test(exact - 1);
        let error = preflight_mutation_plan(122, &compiled.mutation_plan, &txn).unwrap_err();
        assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED");
        assert_eq!(error.limit, Some(u64::try_from(exact - 1).unwrap()));
        assert!(error
            .actual
            .is_some_and(|actual| actual > u64::try_from(exact - 1).unwrap()));
        compiled.mutation_plan.set_work_limit_for_test(exact);
    }
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    let growth_bound = compiled.encoded_growth_bound;
    let undo_bound = compiled.undo_units_bound;
    assert!(growth_bound > 0);
    assert!(undo_bound > 0);
    assert_eq!(
        compile_operations_with_undo_limit(&source, operations(), tiptap_schema(), undo_bound)
            .unwrap()
            .undo_units_bound,
        undo_bound
    );
    let undo_error =
        compile_operations_with_undo_limit(&source, operations(), tiptap_schema(), undo_bound - 1)
            .unwrap_err();
    assert_eq!(undo_error.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(undo_error.actual, Some(undo_bound));

    let fragment = doc.transact().get_xml_fragment("prosemirror").unwrap();
    let mut undo = UndoManager::<()>::new();
    undo.expand_scope(&doc, &fragment);
    let update = {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
        txn.commit();
        txn.encode_update_v1()
    };
    assert!(update.len() <= growth_bound);
    {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        assert_eq!(
            YrsDocumentCodec::new(&schema, &limits)
                .read_json(&fragment, &txn)
                .unwrap(),
            expected
        );
        assert!(
            txn.encode_state_as_update_v1(&StateVector::default()).len()
                <= before_full_len + growth_bound
        );
    }
    let replica = utf16_doc();
    {
        let mut txn = replica.transact_mut();
        txn.apply_update(Update::decode_v1(&before_update).unwrap())
            .unwrap();
        txn.apply_update(Update::decode_v1(&update).unwrap())
            .unwrap();
    }
    {
        let txn = replica.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        assert_eq!(
            YrsDocumentCodec::new(&schema, &limits)
                .read_json(&fragment, &txn)
                .unwrap(),
            expected
        );
    }
    assert!(undo.undo_blocking());
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    assert_eq!(
        YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap(),
        source
    );
}

#[test]
fn outdent_preserves_the_legacy_literal_immediate_parent_list_item_quirk() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "taskList",
            "content": [{
                "type": "taskItem",
                "attrs": { "checked": false },
                "content": [
                    { "type": "paragraph", "content": [{ "type": "text", "text": "parent" }] },
                    {
                        "type": "taskList",
                        "content": [{
                            "type": "taskItem",
                            "attrs": { "checked": true },
                            "content": [{ "type": "paragraph" }]
                        }]
                    }
                ]
            }]
        }]
    });
    let schema = attribute_schema();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let at = u32::try_from(
        crate::render::rendered_text(&document, &schema)
            .chars()
            .count(),
    )
    .unwrap();
    let (_, _, _, compiled) = compile_operations_with_schema(
        &source,
        vec![TypedOperation::OutdentListItem {
            at: point_for_test(at),
        }],
        schema.clone(),
    );
    assert_eq!(to_prosemirror_json(&compiled.preview, &schema), source);
    assert!(compiled.mutation_plan.actions.is_empty());
    assert_eq!(compiled.encoded_growth_bound, 0);
}

#[test]
fn outdent_folds_prior_and_follow_up_text_edits_into_the_moved_item() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [{
                "type": "listItem",
                "content": [
                    { "type": "paragraph", "content": [{ "type": "text", "text": "parent" }] },
                    {
                        "type": "bulletList",
                        "content": [{
                            "type": "listItem",
                            "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "moved" }] }]
                        }]
                    }
                ]
            }]
        }]
    });
    let schema = tiptap_schema();
    let moved = rendered_scalar_offset(&source, &schema, "moved") + 1;
    for operations in [
        vec![
            TypedOperation::InsertText {
                at: point_for_test(moved),
                text: "X".into(),
                marks: vec![],
            },
            TypedOperation::OutdentListItem {
                at: point_for_test(moved),
            },
        ],
        vec![
            TypedOperation::OutdentListItem {
                at: point_for_test(moved),
            },
            TypedOperation::InsertText {
                at: point_for_test(moved),
                text: "X".into(),
                marks: vec![],
            },
        ],
    ] {
        let (doc, schema, limits, compiled) =
            compile_operations_with_schema(&source, operations, tiptap_schema());
        assert!(!compiled
            .mutation_plan
            .actions
            .iter()
            .any(|action| matches!(action, YrsMutationAction::InsertText { .. })));
        let expected = to_prosemirror_json(&compiled.preview, &schema);
        {
            let mut txn = doc.transact_mut();
            execute_mutation_plan(compiled.mutation_plan, &mut txn);
        }
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let actual = YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap();
        assert_eq!(actual, expected);
        assert_eq!(
            actual["content"][0]["content"][1]["content"][0]["content"][0]["text"],
            "mXoved"
        );
    }
}

#[test]
fn outdent_folds_prior_and_follow_up_attrs_into_a_literal_role_based_list_item() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "taskList",
            "attrs": { "listMeta": { "id": "outer" } },
            "content": [{
                "type": "listItem",
                "attrs": { "checked": false, "itemMeta": { "id": "parent" } },
                "content": [
                    { "type": "paragraph", "content": [{ "type": "text", "text": "parent" }] },
                    {
                        "type": "taskList",
                        "attrs": { "listMeta": { "id": "nested" } },
                        "content": [{
                            "type": "listItem",
                            "attrs": { "checked": true, "itemMeta": { "id": "moved" } },
                            "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "moved" }] }]
                        }]
                    }
                ]
            }]
        }]
    });
    let schema = literal_list_item_attr_schema();
    let moved = rendered_scalar_offset(&source, &schema, "moved") + 1;
    let attrs = HashMap::from([
        ("checked".into(), Value::Bool(false)),
        ("itemMeta".into(), json!({ "id": "updated" })),
    ]);
    for operations in [
        vec![
            TypedOperation::UpdateNodeAttrs {
                at: point_for_test(moved),
                attrs: attrs.clone(),
            },
            TypedOperation::OutdentListItem {
                at: point_for_test(moved),
            },
        ],
        vec![
            TypedOperation::OutdentListItem {
                at: point_for_test(moved),
            },
            TypedOperation::UpdateNodeAttrs {
                at: point_for_test(moved),
                attrs: attrs.clone(),
            },
        ],
    ] {
        let (doc, schema, limits, compiled) =
            compile_operations_with_schema(&source, operations, schema.clone());
        assert!(!compiled.mutation_plan.actions.iter().any(|action| {
            matches!(
                action,
                YrsMutationAction::SetXmlAttribute { .. }
                    | YrsMutationAction::RemoveXmlAttribute { .. }
            )
        }));
        let expected = to_prosemirror_json(&compiled.preview, &schema);
        {
            let mut txn = doc.transact_mut();
            execute_mutation_plan(compiled.mutation_plan, &mut txn);
        }
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let actual = YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap();
        assert_eq!(actual, expected);
        let moved_item = &actual["content"][0]["content"][1];
        assert_eq!(moved_item["attrs"]["checked"], false);
        assert_eq!(moved_item["attrs"]["itemMeta"]["id"], "updated");
    }
}

#[test]
fn outdent_then_insert_node_folds_into_the_moved_prepared_item() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [{
                "type": "listItem",
                "content": [
                    { "type": "paragraph", "content": [{ "type": "text", "text": "parent" }] },
                    {
                        "type": "bulletList",
                        "content": [{
                            "type": "listItem",
                            "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "moved" }] }]
                        }]
                    }
                ]
            }]
        }]
    });
    let schema = tiptap_schema();
    let moved = rendered_scalar_offset(&source, &schema, "moved") + 2;
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::OutdentListItem {
                at: point_for_test(moved),
            },
            TypedOperation::InsertNode {
                at: point_for_test(moved),
                node: Node::void("hardBreak".into(), HashMap::new()),
            },
        ],
        schema,
    );
    assert_eq!(
        compiled
            .mutation_plan
            .actions
            .iter()
            .filter(|action| matches!(action, YrsMutationAction::InsertXmlChildren { .. }))
            .count(),
        1
    );
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let actual = YrsDocumentCodec::new(&schema, &limits)
        .read_json(&fragment, &txn)
        .unwrap();
    assert_eq!(actual, expected);
    assert!(actual["content"][0]["content"][1]["content"][0]["content"]
        .as_array()
        .unwrap()
        .iter()
        .any(|node| node["type"] == "hardBreak"));
}

#[test]
fn indent_then_outdent_cancels_the_unchanged_prepared_move() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }] },
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }] }
            ]
        }]
    });
    let schema = tiptap_schema();
    let two = rendered_scalar_offset(&source, &schema, "two") + 1;
    let (_, _, _, compiled) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::IndentListItem {
                at: point_for_test(two),
            },
            TypedOperation::OutdentListItem {
                at: point_for_test(two),
            },
        ],
        schema,
    );
    assert_eq!(
        to_prosemirror_json(&compiled.preview, &tiptap_schema()),
        source
    );
    assert!(compiled.mutation_plan.actions.is_empty());
    assert_eq!(compiled.encoded_growth_bound, 0);
    assert_eq!(compiled.undo_units_bound, 0);
}

#[test]
fn indent_edit_then_outdent_reinserts_only_the_changed_prepared_item() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }] },
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }] }
            ]
        }]
    });
    let schema = tiptap_schema();
    let two = rendered_scalar_offset(&source, &schema, "two") + 1;
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::IndentListItem {
                at: point_for_test(two),
            },
            TypedOperation::InsertText {
                at: point_for_test(two),
                text: "X".into(),
                marks: vec![],
            },
            TypedOperation::OutdentListItem {
                at: point_for_test(two),
            },
        ],
        schema,
    );
    assert!(matches!(
        compiled.mutation_plan.actions.as_slice(),
        [
            YrsMutationAction::DeleteXmlChildren { .. },
            YrsMutationAction::InsertXmlChildren { .. }
        ]
    ));
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let actual = YrsDocumentCodec::new(&schema, &limits)
        .read_json(&fragment, &txn)
        .unwrap();
    assert_eq!(actual, expected);
    assert_eq!(
        actual["content"][0]["content"][1]["content"][0]["content"][0]["text"],
        "tXwo"
    );
}

#[test]
fn outdent_from_a_multi_item_prepared_nested_list_retains_its_prefix_blueprint() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }] },
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }] },
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "three" }] }] }
            ]
        }]
    });
    let schema = tiptap_schema();
    let two = rendered_scalar_offset(&source, &schema, "two") + 1;
    let three = rendered_scalar_offset(&source, &schema, "three") + 1;
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![
            TypedOperation::IndentListItem {
                at: point_for_test(two),
            },
            TypedOperation::IndentListItem {
                at: point_for_test(three),
            },
            TypedOperation::OutdentListItem {
                at: point_for_test(three),
            },
        ],
    );
    assert_eq!(actual, expected);
    let outer = actual["content"][0]["content"].as_array().unwrap();
    assert_eq!(outer.len(), 2);
    assert_eq!(
        outer[0]["content"][1]["content"][0]["content"][0]["content"][0]["text"],
        "two"
    );
    assert_eq!(outer[1]["content"][0]["content"][0]["text"], "three");
}

#[test]
fn outdent_rewrites_a_fully_prepared_outer_list_and_parent_batch() {
    let source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "one" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }
        ]
    });
    let schema = tiptap_schema();
    let two = rendered_scalar_offset(&source, &schema, "two");
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::WrapInList {
                range: range_for_test(0, two + 3),
                list_type: "bulletList".into(),
                item_type: "listItem".into(),
                attrs: HashMap::new(),
                item_attrs: HashMap::new(),
            },
            TypedOperation::IndentListItem {
                at: point_for_test(two + 1),
            },
            TypedOperation::OutdentListItem {
                at: point_for_test(two + 1),
            },
        ],
        schema,
    );
    assert_eq!(
        compiled
            .mutation_plan
            .actions
            .iter()
            .filter(|action| matches!(action, YrsMutationAction::InsertXmlChildren { .. }))
            .count(),
        1
    );
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    assert_eq!(
        YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap(),
        expected
    );
}

#[test]
fn outdent_first_nested_item_under_a_prepared_parent_rewrites_the_owned_batch() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [{
                "type": "listItem",
                "content": [
                    { "type": "paragraph", "content": [{ "type": "text", "text": "outer" }] },
                    {
                        "type": "bulletList",
                        "content": [{
                            "type": "listItem",
                            "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "inner" }] }]
                        }]
                    }
                ]
            }]
        }]
    });
    let schema = tiptap_schema();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&document, &schema);
    let outer = u32::try_from(rendered[..rendered.find("outer").unwrap()].chars().count()).unwrap();
    let inner = u32::try_from(rendered[..rendered.find("inner").unwrap()].chars().count()).unwrap();
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::SplitBlock {
                at: point_for_test(outer + 2),
                node_type: "paragraph".into(),
                attrs: HashMap::new(),
            },
            TypedOperation::OutdentListItem {
                at: point_for_test(inner + 1),
            },
        ],
        schema,
    );
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let actual = YrsDocumentCodec::new(&schema, &limits)
        .read_json(&fragment, &txn)
        .unwrap();
    assert_eq!(actual, expected);
    let items = actual["content"][0]["content"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(items[0]["content"][0]["content"][0]["text"], "ou");
    assert_eq!(items[1]["content"][0]["content"][0]["text"], "ter");
    assert_eq!(items[2]["content"][0]["content"][0]["text"], "inner");
}

#[test]
fn outdent_first_of_multiple_nested_items_under_a_prepared_parent_keeps_the_tail() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [{
                "type": "listItem",
                "content": [
                    { "type": "paragraph", "content": [{ "type": "text", "text": "outer" }] },
                    {
                        "type": "bulletList",
                        "content": [
                            { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "inner" }] }] },
                            { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }] }
                        ]
                    }
                ]
            }]
        }]
    });
    let schema = tiptap_schema();
    let outer = rendered_scalar_offset(&source, &schema, "outer");
    let inner = rendered_scalar_offset(&source, &schema, "inner");
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![
            TypedOperation::SplitBlock {
                at: point_for_test(outer + 2),
                node_type: "paragraph".into(),
                attrs: HashMap::new(),
            },
            TypedOperation::OutdentListItem {
                at: point_for_test(inner + 1),
            },
        ],
    );
    assert_eq!(actual, expected);
    let items = actual["content"][0]["content"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(items[2]["content"][0]["content"][0]["text"], "inner");
    assert_eq!(
        items[2]["content"][1]["content"][0]["content"][0]["content"][0]["text"],
        "tail"
    );
}

#[test]
fn unwrap_first_of_multiple_nested_items_under_a_prepared_parent_keeps_the_tail() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [{
                "type": "listItem",
                "content": [
                    { "type": "paragraph", "content": [{ "type": "text", "text": "outer" }] },
                    {
                        "type": "bulletList",
                        "content": [
                            { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "inner" }] }] },
                            { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }] }
                        ]
                    }
                ]
            }]
        }]
    });
    let schema = tiptap_schema();
    let outer = rendered_scalar_offset(&source, &schema, "outer");
    let inner = rendered_scalar_offset(&source, &schema, "inner");
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![
            TypedOperation::SplitBlock {
                at: point_for_test(outer + 2),
                node_type: "paragraph".into(),
                attrs: HashMap::new(),
            },
            TypedOperation::UnwrapFromList {
                at: point_for_test(inner + 1),
            },
        ],
    );
    assert_eq!(actual, expected);
    let outer_items = actual["content"][0]["content"].as_array().unwrap();
    assert_eq!(outer_items.len(), 2);
    let split_outer = outer_items[1]["content"].as_array().unwrap();
    assert_eq!(split_outer[0]["content"][0]["text"], "ter");
    assert_eq!(split_outer[1]["content"][0]["text"], "inner");
    assert_eq!(
        split_outer[2]["content"][0]["content"][0]["content"][0]["text"],
        "tail"
    );
}

#[test]
fn unwrap_first_item_retains_the_stationary_right_list_and_item_identities() {
    let source = json!({
        "type": "doc",
        "content": [
            {
                "type": "bulletList",
                "content": [
                    { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }] },
                    { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }] },
                    { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "three" }] }] }
                ]
            },
            { "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }
        ]
    });
    let schema = tiptap_schema();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&document, &schema);
    let one = u32::try_from(rendered[..rendered.find("one").unwrap()].chars().count()).unwrap();
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![TypedOperation::UnwrapFromList {
            at: point_for_test(one + 1),
        }],
        schema,
    );
    let (list_id, remaining_item_ids, tail_id) = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let XmlOut::Element(list) = fragment.get(&txn, 0).unwrap() else {
            panic!("list expected")
        };
        let items = list.children(&txn).collect::<Vec<_>>();
        (
            AsRef::<Branch>::as_ref(&list).id(),
            vec![items[1].id(), items[2].id()],
            fragment.get(&txn, 1).unwrap().id(),
        )
    };
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let XmlOut::Element(list) = fragment.get(&txn, 1).unwrap() else {
        panic!("remaining list expected")
    };
    assert_eq!(AsRef::<Branch>::as_ref(&list).id(), list_id);
    assert_eq!(
        list.children(&txn)
            .map(|child| child.id())
            .collect::<Vec<_>>(),
        remaining_item_ids
    );
    assert_eq!(fragment.get(&txn, 2).unwrap().id(), tail_id);
    assert_eq!(
        YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap(),
        expected
    );
}

#[test]
fn unwrap_first_then_insert_node_into_extracted_block() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [
                {
                    "type": "listItem",
                    "content": [{
                        "type": "paragraph",
                        "content": [{ "type": "text", "text": "one" }]
                    }]
                },
                {
                    "type": "listItem",
                    "content": [{
                        "type": "paragraph",
                        "content": [{ "type": "text", "text": "two" }]
                    }]
                }
            ]
        }]
    });
    let schema = tiptap_schema();
    let one = rendered_scalar_offset(&source, &schema, "one");
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::UnwrapFromList {
                at: point_for_test(one + 1),
            },
            TypedOperation::InsertNode {
                at: point_for_test(one + 1),
                node: Node::void("hardBreak".into(), HashMap::new()),
            },
        ],
        schema,
    );
    assert_eq!(
        compiled
            .mutation_plan
            .actions
            .iter()
            .filter(|action| matches!(action, YrsMutationAction::InsertXmlChildren { .. }))
            .count(),
        1,
        "the inline node should be owned by the prepared extracted block"
    );
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let actual = YrsDocumentCodec::new(&schema, &limits)
        .read_json(&fragment, &txn)
        .unwrap();
    assert_eq!(actual, expected);
    assert!(actual["content"][0]["content"]
        .as_array()
        .unwrap()
        .iter()
        .any(|node| node["type"] == "hardBreak"));
}

#[test]
fn unwrap_last_and_middle_retain_the_left_list_and_stationary_item_identities() {
    for selected in [1usize, 2usize] {
        let source = json!({
            "type": "doc",
            "content": [
                {
                    "type": "bulletList",
                    "content": [
                        { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }] },
                        { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }] },
                        { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "three" }] }] }
                    ]
                },
                { "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }
            ]
        });
        let selected_text = if selected == 1 { "two" } else { "three" };
        let schema = tiptap_schema();
        let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
        let rendered = crate::render::rendered_text(&document, &schema);
        let at = u32::try_from(
            rendered[..rendered.find(selected_text).unwrap()]
                .chars()
                .count(),
        )
        .unwrap();
        let (doc, schema, limits, compiled) = compile_operations_with_schema(
            &source,
            vec![TypedOperation::UnwrapFromList {
                at: point_for_test(at + 1),
            }],
            schema,
        );
        let (list_id, item_ids, tail_id) = {
            let txn = doc.transact();
            let fragment = txn.get_xml_fragment("prosemirror").unwrap();
            let XmlOut::Element(list) = fragment.get(&txn, 0).unwrap() else {
                panic!("list expected")
            };
            (
                AsRef::<Branch>::as_ref(&list).id(),
                list.children(&txn)
                    .map(|child| child.id())
                    .collect::<Vec<_>>(),
                fragment.get(&txn, 1).unwrap().id(),
            )
        };
        let expected = to_prosemirror_json(&compiled.preview, &schema);
        {
            let mut txn = doc.transact_mut();
            execute_mutation_plan(compiled.mutation_plan, &mut txn);
        }
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let XmlOut::Element(left_list) = fragment.get(&txn, 0).unwrap() else {
            panic!("left list expected")
        };
        assert_eq!(AsRef::<Branch>::as_ref(&left_list).id(), list_id);
        let retained = left_list
            .children(&txn)
            .map(|child| child.id())
            .collect::<Vec<_>>();
        assert_eq!(retained, item_ids[..selected]);
        let tail_index = if selected == 1 { 3 } else { 2 };
        assert_eq!(fragment.get(&txn, tail_index).unwrap().id(), tail_id);
        assert_eq!(
            YrsDocumentCodec::new(&schema, &limits)
                .read_json(&fragment, &txn)
                .unwrap(),
            expected
        );
    }
}

#[test]
fn unwrap_middle_retains_the_larger_stationary_side_with_deterministic_left_ties() {
    let source = json!({
        "type": "doc",
        "content": [
            {
                "type": "bulletList",
                "content": [
                    { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }] },
                    { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }] },
                    { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "three" }] }] },
                    { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "four" }] }] },
                    { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "five" }] }] }
                ]
            },
            { "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }
        ]
    });
    for (selected, selected_text, retains_right, sticky_item) in [
        (1usize, "two", true, 2usize),
        (3usize, "four", false, 2usize),
        (2usize, "three", false, 1usize),
    ] {
        let schema = tiptap_schema();
        let at = rendered_scalar_offset(&source, &schema, selected_text);
        let (doc, schema, limits, compiled) = compile_operations_with_schema(
            &source,
            vec![TypedOperation::UnwrapFromList {
                at: point_for_test(at + 1),
            }],
            schema,
        );
        let (
            list_id,
            item_ids,
            stationary_text_id,
            stationary_sticky,
            tail_id,
            tail_text_id,
            tail_sticky,
        ) = {
            let txn = doc.transact();
            let fragment = txn.get_xml_fragment("prosemirror").unwrap();
            let XmlOut::Element(list) = fragment.get(&txn, 0).unwrap() else {
                panic!("list expected")
            };
            let items = list.children(&txn).collect::<Vec<_>>();
            let stationary_text = list_item_text(&items[sticky_item], &txn);
            let tail_text = paragraph_text(&fragment, &txn, 1);
            (
                AsRef::<Branch>::as_ref(&list).id(),
                items.iter().map(XmlOut::id).collect::<Vec<_>>(),
                <XmlTextRef as AsRef<Branch>>::as_ref(&stationary_text).id(),
                StickyIndex::at(
                    &txn,
                    BranchPtr::from(<XmlTextRef as AsRef<Branch>>::as_ref(&stationary_text)),
                    1,
                    Assoc::After,
                )
                .unwrap(),
                fragment.get(&txn, 1).unwrap().id(),
                <XmlTextRef as AsRef<Branch>>::as_ref(&tail_text).id(),
                StickyIndex::at(
                    &txn,
                    BranchPtr::from(<XmlTextRef as AsRef<Branch>>::as_ref(&tail_text)),
                    2,
                    Assoc::After,
                )
                .unwrap(),
            )
        };
        let expected = to_prosemirror_json(&compiled.preview, &schema);
        {
            let mut txn = doc.transact_mut();
            execute_mutation_plan(compiled.mutation_plan, &mut txn);
        }
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let stationary_root_index = if retains_right { 2 } else { 0 };
        let XmlOut::Element(stationary_list) = fragment.get(&txn, stationary_root_index).unwrap()
        else {
            panic!("stationary list expected")
        };
        assert_eq!(AsRef::<Branch>::as_ref(&stationary_list).id(), list_id);
        let expected_ids = if retains_right {
            &item_ids[selected + 1..]
        } else {
            &item_ids[..selected]
        };
        assert_eq!(
            stationary_list
                .children(&txn)
                .map(|child| child.id())
                .collect::<Vec<_>>(),
            expected_ids
        );
        let resolved_stationary = stationary_sticky.get_offset(&txn).unwrap();
        assert_eq!(resolved_stationary.branch.id(), stationary_text_id);
        assert_eq!(resolved_stationary.index, 1);
        assert_eq!(fragment.get(&txn, 3).unwrap().id(), tail_id);
        let resolved_tail = tail_sticky.get_offset(&txn).unwrap();
        assert_eq!(resolved_tail.branch.id(), tail_text_id);
        assert_eq!(resolved_tail.index, 2);
        assert_eq!(
            YrsDocumentCodec::new(&schema, &limits)
                .read_json(&fragment, &txn)
                .unwrap(),
            expected
        );
    }
}

#[test]
fn unwrap_middle_then_edit_retained_right_item_and_tail() {
    let source = json!({
        "type": "doc",
        "content": [
            {
                "type": "bulletList",
                "content": [
                    { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }] },
                    { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }] },
                    { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "three" }] }] }
                ]
            },
            { "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }
        ]
    });
    let schema = tiptap_schema();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&document, &schema);
    let offset = |needle: &str| {
        u32::try_from(rendered[..rendered.find(needle).unwrap()].chars().count()).unwrap()
    };
    let two = offset("two");
    let three = offset("three");
    let tail = offset("tail");
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::UnwrapFromList {
                at: point_for_test(two + 1),
            },
            TypedOperation::InsertText {
                at: point_for_test(three + 1),
                text: "X".into(),
                marks: vec![],
            },
            TypedOperation::InsertText {
                at: point_for_test(tail + 2),
                text: "Y".into(),
                marks: vec![],
            },
        ],
        schema,
    );
    assert_eq!(
        compiled
            .mutation_plan
            .actions
            .iter()
            .filter(|action| matches!(action, YrsMutationAction::InsertText { .. }))
            .count(),
        1,
        "the prepared right item edit should fold, leaving only the tail text action"
    );
    let tail_id = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        fragment.get(&txn, 1).unwrap().id()
    };
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    assert_eq!(fragment.get(&txn, 3).unwrap().id(), tail_id);
    let actual = YrsDocumentCodec::new(&schema, &limits)
        .read_json(&fragment, &txn)
        .unwrap();
    assert_eq!(actual, expected);
    assert_eq!(
        actual["content"][2]["content"][0]["content"][0]["content"][0]["text"],
        "tXhree"
    );
    assert_eq!(actual["content"][3]["content"][0]["text"], "taYil");
}

#[test]
fn unwrap_nested_list_item_splices_blocks_inside_the_outer_item() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [{
                "type": "listItem",
                "content": [
                    { "type": "paragraph", "content": [{ "type": "text", "text": "outer" }] },
                    {
                        "type": "bulletList",
                        "content": [
                            { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "inner-one" }] }] },
                            { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "inner-two" }] }] }
                        ]
                    }
                ]
            }]
        }]
    });
    let schema = tiptap_schema();
    let inner = rendered_scalar_offset(&source, &schema, "inner-one");
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![TypedOperation::UnwrapFromList {
            at: point_for_test(inner + 1),
        }],
    );
    assert_eq!(actual, expected);
    let outer_content = actual["content"][0]["content"][0]["content"]
        .as_array()
        .unwrap();
    assert_eq!(outer_content[0]["content"][0]["text"], "outer");
    assert_eq!(outer_content[1]["content"][0]["text"], "inner-one");
    assert_eq!(outer_content[2]["type"], "bulletList");
    assert_eq!(
        outer_content[2]["content"][0]["content"][0]["content"][0]["text"],
        "inner-two"
    );
}

#[test]
fn multiple_sibling_unwraps_preflight_in_both_operation_orders() {
    let source = json!({
        "type": "doc",
        "content": [
            {
                "type": "bulletList",
                "content": [{
                    "type": "listItem",
                    "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }]
                }]
            },
            {
                "type": "bulletList",
                "content": [{
                    "type": "listItem",
                    "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }]
                }]
            }
        ]
    });
    let schema = tiptap_schema();
    let one = rendered_scalar_offset(&source, &schema, "one") + 1;
    let two = rendered_scalar_offset(&source, &schema, "two") + 1;
    for positions in [[one, two], [two, one]] {
        let (actual, expected, _, _, _) = compile_and_execute(
            source.clone(),
            positions
                .into_iter()
                .map(|at| TypedOperation::UnwrapFromList {
                    at: point_for_test(at),
                })
                .collect(),
        );
        assert_eq!(actual, expected);
        assert_eq!(
            actual["content"]
                .as_array()
                .unwrap()
                .iter()
                .map(|node| node["type"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["paragraph", "paragraph"]
        );
    }
}

#[test]
fn unwrap_nested_list_under_a_prepared_parent_rewrites_one_owned_batch() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [{
                "type": "listItem",
                "content": [
                    { "type": "paragraph", "content": [{ "type": "text", "text": "outer" }] },
                    {
                        "type": "bulletList",
                        "content": [{
                            "type": "listItem",
                            "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "inner" }] }]
                        }]
                    }
                ]
            }]
        }]
    });
    let schema = tiptap_schema();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&document, &schema);
    let outer = u32::try_from(rendered[..rendered.find("outer").unwrap()].chars().count()).unwrap();
    let inner = u32::try_from(rendered[..rendered.find("inner").unwrap()].chars().count()).unwrap();
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::SplitBlock {
                at: point_for_test(outer + 2),
                node_type: "paragraph".into(),
                attrs: HashMap::new(),
            },
            TypedOperation::UnwrapFromList {
                at: point_for_test(inner + 1),
            },
        ],
        schema,
    );
    assert_eq!(
        compiled
            .mutation_plan
            .actions
            .iter()
            .filter(|action| matches!(action, YrsMutationAction::InsertXmlChildren { .. }))
            .count(),
        1
    );
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let actual = YrsDocumentCodec::new(&schema, &limits)
        .read_json(&fragment, &txn)
        .unwrap();
    assert_eq!(actual, expected);
    let prepared_item = &actual["content"][0]["content"][1];
    assert_eq!(prepared_item["content"][1]["type"], "paragraph");
    assert_eq!(prepared_item["content"][1]["content"][0]["text"], "inner");
}

#[test]
fn unwrap_supports_empty_items_and_preserves_void_and_task_attrs() {
    let empty = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [{
                "type": "listItem",
                "content": [{ "type": "paragraph" }]
            }]
        }]
    });
    let schema = tiptap_schema();
    let empty_document = from_prosemirror_json(&empty, &schema, UnknownTypeMode::Preserve).unwrap();
    let empty_at = u32::try_from(
        crate::render::rendered_text(&empty_document, &schema)
            .chars()
            .count(),
    )
    .unwrap();
    let (actual_empty, expected_empty, _, _, _) = compile_and_execute(
        empty,
        vec![TypedOperation::UnwrapFromList {
            at: point_for_test(empty_at),
        }],
    );
    assert_eq!(actual_empty, expected_empty);
    assert_eq!(
        actual_empty,
        json!({ "type": "doc", "content": [{ "type": "paragraph" }] })
    );

    let task = json!({
        "type": "doc",
        "content": [{
            "type": "taskList",
            "attrs": { "listMeta": { "owner": "team", "rank": 7 } },
            "content": [
                {
                    "type": "taskItem",
                    "attrs": { "checked": true, "itemMeta": { "id": "extract" } },
                    "content": [
                        { "type": "paragraph", "content": [{ "type": "text", "text": "extract" }] },
                        { "type": "image", "attrs": { "src": "asset://one", "alt": "typed" } }
                    ]
                },
                {
                    "type": "taskItem",
                    "attrs": { "checked": false, "itemMeta": { "id": "stationary", "score": 4 } },
                    "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "remain" }] }]
                }
            ]
        }]
    });
    let schema = attribute_schema();
    let extract = rendered_scalar_offset(&task, &schema, "extract");
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &task,
        vec![TypedOperation::UnwrapFromList {
            at: point_for_test(extract + 1),
        }],
        schema,
    );
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let actual = YrsDocumentCodec::new(&schema, &limits)
        .read_json(&fragment, &txn)
        .unwrap();
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][1]["type"], "image");
    assert_eq!(actual["content"][1]["attrs"]["src"], "asset://one");
    assert_eq!(actual["content"][1]["attrs"]["alt"], "typed");
    assert_eq!(
        actual["content"][2]["attrs"]["listMeta"],
        json!({ "owner": "team", "rank": 7 })
    );
    assert_eq!(
        actual["content"][2]["content"][0]["attrs"]["checked"],
        false
    );
    assert_eq!(
        actual["content"][2]["content"][0]["attrs"]["itemMeta"],
        json!({ "id": "stationary", "score": 4 })
    );
}

#[test]
fn unwrap_preflight_growth_undo_and_replica_bounds_are_exactly_enforced() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [{
                "type": "listItem",
                "content": [{
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": "one" }]
                }]
            }]
        }]
    });
    let schema = tiptap_schema();
    let one = rendered_scalar_offset(&source, &schema, "one");
    let operations = || {
        vec![TypedOperation::UnwrapFromList {
            at: point_for_test(one + 1),
        }]
    };
    let (doc, schema, limits, mut compiled) =
        compile_operations_with_schema(&source, operations(), schema);
    let (before_update, before_full_len) = {
        let txn = doc.transact();
        let update = txn.encode_state_as_update_v1(&StateVector::default());
        let len = update.len();
        (update, len)
    };
    {
        let txn = doc.transact();
        let preflight =
            preflight_mutation_work_for_test(122, &compiled.mutation_plan, &txn).unwrap();
        let exact = compiled.mutation_plan.compilation_work_for_test() + preflight;
        let exact_u64 = u64::try_from(exact).unwrap();
        compiled.mutation_plan.set_work_limit_for_test(exact);
        preflight_mutation_plan(122, &compiled.mutation_plan, &txn).unwrap();
        compiled.mutation_plan.set_work_limit_for_test(exact - 1);
        let error = preflight_mutation_plan(122, &compiled.mutation_plan, &txn).unwrap_err();
        assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED");
        assert_eq!(error.limit, Some(exact_u64 - 1));
        assert!(error.actual.is_some_and(|actual| actual > exact_u64 - 1));
        compiled.mutation_plan.set_work_limit_for_test(exact);
    }
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    let growth_bound = compiled.encoded_growth_bound;
    let undo_bound = compiled.undo_units_bound;
    assert!(undo_bound > 0);
    assert_eq!(
        compile_operations_with_undo_limit(&source, operations(), tiptap_schema(), undo_bound,)
            .unwrap()
            .undo_units_bound,
        undo_bound
    );
    let undo_error =
        compile_operations_with_undo_limit(&source, operations(), tiptap_schema(), undo_bound - 1)
            .unwrap_err();
    assert_eq!(undo_error.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(undo_error.limit, Some(undo_bound - 1));
    assert_eq!(undo_error.actual, Some(undo_bound));

    let fragment = doc.transact().get_xml_fragment("prosemirror").unwrap();
    let mut undo = UndoManager::<()>::new();
    undo.expand_scope(&doc, &fragment);
    let update = {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
        txn.commit();
        txn.encode_update_v1()
    };
    assert!(
        update.len() <= growth_bound,
        "{} > {growth_bound}",
        update.len()
    );
    {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        assert_eq!(
            YrsDocumentCodec::new(&schema, &limits)
                .read_json(&fragment, &txn)
                .unwrap(),
            expected
        );
        let after_full_len = txn.encode_state_as_update_v1(&StateVector::default()).len();
        assert!(after_full_len <= before_full_len + growth_bound);
    }

    let replica = utf16_doc();
    {
        let mut txn = replica.transact_mut();
        txn.apply_update(Update::decode_v1(&before_update).unwrap())
            .unwrap();
        txn.apply_update(Update::decode_v1(&update).unwrap())
            .unwrap();
    }
    {
        let txn = replica.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        assert_eq!(
            YrsDocumentCodec::new(&schema, &limits)
                .read_json(&fragment, &txn)
                .unwrap(),
            expected
        );
    }

    assert!(undo.undo_blocking());
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    assert_eq!(
        YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap(),
        source
    );
}

#[test]
fn wrap_multiple_remaps_following_attrs_and_structural_insertions() {
    let source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "aa" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "bb" }] },
            { "type": "h2", "attrs": { "id": "tail" }, "content": [{ "type": "text", "text": "cc" }] }
        ]
    });
    let schema = attribute_schema();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&document, &schema);
    let tail_byte = rendered.find("cc").unwrap();
    let tail = u32::try_from(rendered[..tail_byte].chars().count()).unwrap();
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::WrapInList {
                range: range_for_test(0, tail - 1),
                list_type: "taskList".into(),
                item_type: "taskItem".into(),
                attrs: HashMap::new(),
                item_attrs: HashMap::new(),
            },
            TypedOperation::UpdateNodeAttrs {
                at: point_for_test(tail),
                attrs: HashMap::from([("id".into(), Value::String("tail-new".into()))]),
            },
            TypedOperation::InsertNode {
                at: point_for_test(tail + 2),
                node: Node::void("hardBreak".into(), HashMap::new()),
            },
        ],
        schema,
    );
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let actual = YrsDocumentCodec::new(&schema, &limits)
        .read_json(&fragment, &txn)
        .unwrap();
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][1]["attrs"]["id"], "tail-new");
    assert!(actual["content"][1]["content"]
        .as_array()
        .unwrap()
        .iter()
        .any(|node| node["type"] == "hardBreak"));
}

#[test]
fn split_sequences_materialize_only_the_compact_final_plan() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "A😀B" }]
        }]
    });
    let (_, _, _, inserted) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::InsertText {
                at: point_for_test(3),
                text: "X".into(),
                marks: vec![],
            },
            TypedOperation::SplitBlock {
                at: point_for_test(2),
                node_type: "paragraph".into(),
                attrs: HashMap::new(),
            },
        ],
        tiptap_schema(),
    );
    assert!(!inserted
        .mutation_plan
        .actions
        .iter()
        .any(|action| matches!(action, YrsMutationAction::InsertText { .. })));
    let inserted_nodes = inserted
        .mutation_plan
        .actions
        .iter()
        .find_map(|action| match action {
            YrsMutationAction::InsertXmlChildren { nodes, .. } => Some(nodes),
            _ => None,
        })
        .unwrap();
    let PreparedXmlNode::Element { children, .. } = &inserted_nodes[0].node else {
        panic!("prepared right block expected")
    };
    let PreparedXmlNode::Text { runs } = &children[0].node else {
        panic!("prepared right text expected")
    };
    assert_eq!(prepared_text_for_test(runs), "BX");

    let (_, _, _, marked) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::AddMark {
                range: range_for_test(2, 3),
                mark: Mark::new("bold".into(), HashMap::new()),
            },
            TypedOperation::SplitBlock {
                at: point_for_test(2),
                node_type: "paragraph".into(),
                attrs: HashMap::new(),
            },
        ],
        tiptap_schema(),
    );
    assert!(!marked
        .mutation_plan
        .actions
        .iter()
        .any(|action| matches!(action, YrsMutationAction::FormatText { .. })));
    let marked_nodes = marked
        .mutation_plan
        .actions
        .iter()
        .find_map(|action| match action {
            YrsMutationAction::InsertXmlChildren { nodes, .. } => Some(nodes),
            _ => None,
        })
        .unwrap();
    let PreparedXmlNode::Element { children, .. } = &marked_nodes[0].node else {
        panic!("prepared marked block expected")
    };
    let PreparedXmlNode::Text { runs } = &children[0].node else {
        panic!("prepared marked text expected")
    };
    assert_eq!(prepared_text_for_test(runs), "B");
    assert_eq!(runs[0].attrs.get("bold"), Some(&Any::Bool(true)));

    let heading = json!({
        "type": "doc",
        "content": [{
            "type": "h2",
            "attrs": { "id": "left" },
            "content": [{ "type": "text", "text": "abcd" }]
        }]
    });
    let (_, _, _, attributed) = compile_operations_with_schema(
        &heading,
        vec![
            TypedOperation::SplitBlock {
                at: point_for_test(2),
                node_type: "h2".into(),
                attrs: HashMap::from([("id".into(), Value::String("right-old".into()))]),
            },
            TypedOperation::UpdateNodeAttrs {
                at: point_for_test(2),
                attrs: HashMap::from([("id".into(), Value::String("right-new".into()))]),
            },
        ],
        attribute_schema(),
    );
    assert!(!attributed.mutation_plan.actions.iter().any(|action| {
        matches!(
            action,
            YrsMutationAction::SetXmlAttribute { .. }
                | YrsMutationAction::RemoveXmlAttribute { .. }
        )
    }));
    let attributed_nodes = attributed
        .mutation_plan
        .actions
        .iter()
        .find_map(|action| match action {
            YrsMutationAction::InsertXmlChildren { nodes, .. } => Some(nodes),
            _ => None,
        })
        .unwrap();
    let PreparedXmlNode::Element { attrs, .. } = &attributed_nodes[0].node else {
        panic!("prepared attributed block expected")
    };
    assert!(attrs
        .iter()
        .any(|(key, value)| { key == "id" && value == &Any::String("right-new".into()) }));
}

#[test]
fn folded_split_undo_bound_uses_the_compact_final_plan_exactly() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "A😀B" }]
        }]
    });
    let operations = || {
        vec![
            TypedOperation::InsertText {
                at: point_for_test(3),
                text: "X".into(),
                marks: vec![],
            },
            TypedOperation::SplitBlock {
                at: point_for_test(2),
                node_type: "paragraph".into(),
                attrs: HashMap::new(),
            },
        ]
    };
    let exact =
        compile_operations_with_undo_limit(&source, operations(), tiptap_schema(), u64::MAX)
            .unwrap()
            .undo_units_bound;
    assert!(exact > 0);
    let accepted =
        compile_operations_with_undo_limit(&source, operations(), tiptap_schema(), exact).unwrap();
    assert_eq!(accepted.undo_units_bound, exact);
    let rejected =
        compile_operations_with_undo_limit(&source, operations(), tiptap_schema(), exact - 1)
            .unwrap_err();
    assert_eq!(rejected.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(rejected.limit, Some(exact - 1));
    assert_eq!(rejected.actual, Some(exact));

    let plain = compile_operations_with_undo_limit(
        &source,
        vec![TypedOperation::InsertText {
            at: point_for_test(1),
            text: "x".into(),
            marks: vec![],
        }],
        tiptap_schema(),
        1,
    )
    .unwrap();
    assert_eq!(plain.undo_units_bound, 1);

    let emoji = compile_operations_with_undo_limit(
        &source,
        vec![TypedOperation::InsertText {
            at: point_for_test(1),
            text: "🙂".into(),
            marks: vec![],
        }],
        tiptap_schema(),
        2,
    )
    .unwrap();
    assert_eq!(emoji.undo_units_bound, 2);

    let (emoji_doc, _, _, emoji_compiled) = compile_operations_with_schema(
        &source,
        vec![TypedOperation::InsertText {
            at: point_for_test(1),
            text: "🙂".into(),
            marks: vec![],
        }],
        tiptap_schema(),
    );
    let fragment = emoji_doc
        .transact()
        .get_xml_fragment("prosemirror")
        .unwrap();
    let mut undo = UndoManager::<()>::new();
    undo.expand_scope(&emoji_doc, &fragment);
    {
        let mut txn = emoji_doc.transact_mut();
        execute_mutation_plan(emoji_compiled.mutation_plan, &mut txn);
    }
    let inserted = undo.undo_stack()[0]
        .insertions()
        .iter()
        .flat_map(|(_, ranges)| ranges.into_iter())
        .map(|range| u64::from(range.end - range.start))
        .sum::<u64>();
    assert_eq!(inserted, 2);
    assert!(inserted <= emoji_compiled.undo_units_bound);
}

#[test]
fn preflight_rejects_same_utf16_length_text_or_mark_changes() {
    fn compiled_insert(
        source: &Value,
    ) -> (
        Doc,
        crate::schema::Schema,
        ResourceLimits,
        CompiledTransaction,
    ) {
        let (doc, schema, limits, editing_limits, document) = diagnostic_doc(source);
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
                    request_id: 124,
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
        (doc, schema, limits, compiled)
    }

    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "ab" }]
        }]
    });
    let (text_doc, _, _, text_compiled) = compiled_insert(&source);
    {
        let mut txn = text_doc.transact_mut();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let text = paragraph_text(&fragment, &txn, 0);
        text.remove_range(&mut txn, 0, 1);
        text.insert(&mut txn, 0, "z");
    }
    let txn = text_doc.transact();
    assert_eq!(
        preflight_mutation_plan(124, &text_compiled.mutation_plan, &txn)
            .unwrap_err()
            .code,
        "ENGINE_INVARIANT_FAILED"
    );

    let (mark_doc, _, _, mark_compiled) = compiled_insert(&source);
    {
        let mut txn = mark_doc.transact_mut();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let text = paragraph_text(&fragment, &txn, 0);
        text.format(
            &mut txn,
            0,
            1,
            Attrs::from([(Arc::<str>::from("bold"), Any::Bool(true))]),
        );
    }
    let txn = mark_doc.transact();
    assert_eq!(
        preflight_mutation_plan(124, &mark_compiled.mutation_plan, &txn)
            .unwrap_err()
            .code,
        "ENGINE_INVARIANT_FAILED"
    );
}

#[test]
fn crdt_envelope_bounds_fragmented_text_and_deep_xml_actual_costs() {
    fn id_set_units(set: &yrs::IdSet) -> u64 {
        set.iter()
            .flat_map(|(_, ranges)| ranges.into_iter())
            .map(|range| u64::from(range.end - range.start))
            .sum()
    }

    fn assert_actual_costs_bounded(
        source: Value,
        operations: Vec<TypedOperation>,
        schema: crate::schema::Schema,
        expect_legacy_underbound: bool,
    ) {
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
        // Populate unrelated live and disjoint deleted clocks from independent
        // clients. They are outside the editor fragment but part of the same
        // transaction-wide Yrs snapshot envelope.
        for client in 10..14u64 {
            let auxiliary = Doc::with_client_id(client);
            let update = {
                let text = auxiliary.get_or_insert_text(format!("aux-{client}"));
                let mut txn = auxiliary.transact_mut();
                text.insert(&mut txn, 0, "abcdef");
                text.remove_range(&mut txn, 1, 1);
                text.remove_range(&mut txn, 3, 1);
                drop(txn);
                auxiliary
                    .transact()
                    .encode_state_as_update_v1(&StateVector::default())
            };
            doc.transact_mut()
                .apply_update(Update::decode_v1(&update).unwrap())
                .unwrap();
        }
        let before_full_len = doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default())
            .len();
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
                    request_id: 125,
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
        let legacy_visible_delete_bound =
            compiled
                .mutation_plan
                .actions
                .iter()
                .fold(0u64, |units, action| match action {
                    YrsMutationAction::DeleteXmlChildren { child_count, .. } => {
                        units + u64::from(*child_count)
                    }
                    YrsMutationAction::DeleteText { len_utf16, .. } => {
                        units + u64::from(*len_utf16)
                    }
                    _ => units,
                });
        let fragment = doc.transact().get_xml_fragment("prosemirror").unwrap();
        let mut undo = UndoManager::<()>::new();
        undo.expand_scope(&doc, &fragment);
        let actual_update = {
            let mut txn = doc.transact_mut();
            execute_mutation_plan(compiled.mutation_plan, &mut txn);
            txn.commit();
            txn.encode_update_v1()
        };
        assert!(actual_update.len() <= compiled.encoded_growth_bound);
        let after_full_len = doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default())
            .len();
        assert!(after_full_len <= before_full_len + compiled.encoded_growth_bound);
        let item = undo.undo_stack().last().expect("mutation must be undoable");
        let actual_insertions = id_set_units(item.insertions());
        let actual_deletions = id_set_units(item.deletions());
        let actual_undo = actual_insertions + actual_deletions;
        if expect_legacy_underbound {
            assert!(
                actual_deletions > legacy_visible_delete_bound,
                "{actual_deletions} <= legacy bound {legacy_visible_delete_bound}"
            );
        } else {
            assert!(actual_insertions >= 6, "{actual_insertions}");
        }
        assert!(
            actual_undo <= compiled.undo_units_bound,
            "actual undo units {actual_undo} exceed bound {}",
            compiled.undo_units_bound
        );
    }

    let rich_runs = (0..256)
        .map(|index| {
            json!({
                "type": "text",
                "text": if index % 2 == 0 { "a" } else { "b" },
                "marks": [{ "type": if index % 2 == 0 { "bold" } else { "italic" } }]
            })
        })
        .collect::<Vec<_>>();
    let fragmented = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "LEFT" }] },
            { "type": "paragraph", "content": rich_runs },
            { "type": "paragraph", "content": [{ "type": "text", "text": "RIGHT" }] }
        ]
    });
    let schema = tiptap_schema();
    let semantic = from_prosemirror_json(&fragmented, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&semantic, &schema);
    let from = u32::try_from(rendered.find("LEFT").unwrap() + 2).unwrap();
    let to = u32::try_from(rendered.find("RIGHT").unwrap() + 2).unwrap();
    assert_actual_costs_bounded(
        fragmented,
        vec![TypedOperation::DeleteRange {
            range: range_for_test(from, to),
        }],
        schema,
        true,
    );

    let prepared_emoji = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "🙂" }]
        }]
    });
    assert_actual_costs_bounded(
        prepared_emoji,
        vec![TypedOperation::WrapInList {
            range: range_for_test(0, 1),
            list_type: "bulletList".into(),
            attrs: HashMap::new(),
            item_type: "listItem".into(),
            item_attrs: HashMap::new(),
        }],
        tiptap_schema(),
        false,
    );
}

#[test]
fn pure_insert_skips_but_plain_mark_add_requires_a_snapshot_envelope() {
    let source = json!({
        "type": "doc",
        "content": (0..256)
            .map(|_| json!({
                "type": "paragraph",
                "content": [{ "type": "text", "text": "abcdefgh" }]
            }))
            .collect::<Vec<_>>()
    });
    let (doc, schema, limits, editing_limits, document) = diagnostic_doc(&source);
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let compiled = compile_transaction_with_yrs(
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
            request_id: 126,
            base_document_revision: 0,
            origin: TransactionOrigin::LocalInput,
            operations: vec![
                TypedOperation::InsertText {
                    at: point_for_test(1),
                    text: "!".into(),
                    marks: vec![],
                },
                TypedOperation::AddMark {
                    range: range_for_test(2, 4),
                    mark: Mark::new("bold".into(), HashMap::new()),
                },
            ],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        },
        &txn,
        &fragment,
    )
    .unwrap();
    assert!(compiled.mutation_plan.requires_crdt_envelope());

    let insert_only = compile_transaction_with_yrs(
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
            request_id: 127,
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
    .unwrap();
    assert!(!insert_only.mutation_plan.requires_crdt_envelope());
}

#[test]
fn pending_crdt_state_rejects_local_compilation_atomically() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "ready" }]
        }]
    });
    let (doc, schema, limits, editing_limits, document) = diagnostic_doc(&source);
    let remote = Doc::with_client_id(77);
    let remote_text = remote.get_or_insert_text("missing-prefix");
    {
        let mut txn = remote.transact_mut();
        remote_text.insert(&mut txn, 0, "a");
    }
    let suffix_update = {
        let mut txn = remote.transact_mut();
        remote_text.insert(&mut txn, 1, "b");
        txn.commit();
        txn.encode_update_v1()
    };
    doc.transact_mut()
        .apply_update(Update::decode_v1(&suffix_update).unwrap())
        .unwrap();
    let txn = doc.transact();
    assert!(txn.store().pending_update().is_some());
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let before = txn.state_vector();
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
            request_id: 128,
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
    .unwrap_err();
    assert_eq!(error.code, "ENGINE_NOT_READY");
    assert_eq!(txn.state_vector(), before);
}

#[test]
fn document_guard_rejects_pending_crdt_state_before_snapshot_validation() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "ready" }]
        }]
    });
    let (doc, _schema, _limits, compiled) = compile_operations_with_schema(
        &source,
        vec![TypedOperation::InsertText {
            at: point_for_test(1),
            text: "!".into(),
            marks: vec![],
        }],
        tiptap_schema(),
    );
    let remote = Doc::with_client_id(78);
    let remote_text = remote.get_or_insert_text("missing-prefix");
    {
        let mut txn = remote.transact_mut();
        remote_text.insert(&mut txn, 0, "a");
    }
    let suffix_update = {
        let mut txn = remote.transact_mut();
        remote_text.insert(&mut txn, 1, "b");
        txn.commit();
        txn.encode_update_v1()
    };
    doc.transact_mut()
        .apply_update(Update::decode_v1(&suffix_update).unwrap())
        .unwrap();
    let txn = doc.transact();
    assert!(txn.store().pending_update().is_some());
    let before = txn.encode_state_as_update_v1(&StateVector::default());
    let error = preflight_mutation_plan(178, &compiled.mutation_plan, &txn).unwrap_err();
    assert_eq!(error.code, "ENGINE_NOT_READY");
    assert_eq!(
        txn.encode_state_as_update_v1(&StateVector::default()),
        before
    );
}

#[test]
fn tombstone_scan_reservation_is_exact_and_compiler_charges_it() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "a" }]
        }]
    });
    let (doc, schema, limits, editing_limits, document) = diagnostic_doc(&source);
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let text = paragraph_text(&fragment, &txn, 0);
        for _ in 0..512 {
            text.insert(&mut txn, 1, "x");
            text.remove_range(&mut txn, 1, 1);
        }
    }
    let txn = doc.transact();
    let exact_clock_work = crdt_clock_scan_reservation(129, &txn, usize::MAX).unwrap();
    assert!(exact_clock_work > 512);
    assert_eq!(
        crdt_clock_scan_reservation(129, &txn, exact_clock_work - 1)
            .unwrap_err()
            .code,
        "OPERATION_LIMIT_EXCEEDED"
    );
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let compiled = compile_transaction_with_yrs(
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
            request_id: 129,
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
    .unwrap();
    assert!(compiled.mutation_plan.scan_work >= exact_clock_work * 2);

    let compile_delete = |resource_limits: &ResourceLimits| {
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        compile_transaction_with_yrs(
            CompilationContext {
                document: &document,
                selection: None,
                schema: &schema,
                resource_limits,
                editing_limits: &editing_limits,
                document_revision: 0,
                max_length: None,
            },
            TypedTransaction {
                request_id: 131,
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
    };
    let deletion = compile_delete(&limits).unwrap();
    assert!(deletion.mutation_plan.requires_crdt_envelope());
    let envelope = crdt_envelope(131, &txn, limits.max_encoded_state_bytes).unwrap();
    let exact_scan_limit = deletion
        .mutation_plan
        .scan_work
        .checked_add(envelope.scan_work)
        .unwrap();
    let mut exact_limits = limits.clone();
    exact_limits.max_input_bytes = exact_scan_limit;
    compile_delete(&exact_limits).unwrap();
    exact_limits.max_input_bytes = exact_scan_limit - 1;
    let one_under = compile_delete(&exact_limits).unwrap_err();
    assert_eq!(one_under.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(
        one_under.limit,
        Some(u64::try_from(exact_scan_limit - 1).unwrap())
    );
    assert_eq!(
        one_under.actual,
        Some(u64::try_from(exact_scan_limit).unwrap())
    );
}

#[test]
fn fully_deleted_clients_and_hidden_format_cleanup_are_bounded() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "ab" }]
        }]
    });
    let (doc, schema, limits, editing_limits, document) = diagnostic_doc(&source);
    for client in 200..204u64 {
        let remote = Doc::with_client_id(client);
        let text = remote.get_or_insert_text(format!("deleted-{client}"));
        {
            let mut txn = remote.transact_mut();
            text.insert(&mut txn, 0, "x");
            text.remove_range(&mut txn, 0, 1);
        }
        let update = remote
            .transact()
            .encode_state_as_update_v1(&StateVector::default());
        doc.transact_mut()
            .apply_update(Update::decode_v1(&update).unwrap())
            .unwrap();
    }
    // Add an adjacent true/null format pair at a zero-width gap. It is
    // semantically invisible but a later full-span format must clean it up.
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let text = paragraph_text(&fragment, &txn, 0);
        text.format(
            &mut txn,
            1,
            0,
            Attrs::from([(Arc::<str>::from("bold"), Any::Bool(true))]),
        );
    }
    let envelope = crdt_envelope(130, &doc.transact(), limits.max_encoded_state_bytes).unwrap();
    assert!(envelope.client_count >= 5);
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
                request_id: 130,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::AddMark {
                    range: range_for_test(0, 2),
                    mark: Mark::new("bold".into(), HashMap::new()),
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
        .unwrap()
    };
    assert!(compiled.mutation_plan.requires_crdt_envelope());
    let before_full_len = doc
        .transact()
        .encode_state_as_update_v1(&StateVector::default())
        .len();
    let fragment = doc.transact().get_xml_fragment("prosemirror").unwrap();
    let mut undo = UndoManager::<()>::new();
    undo.expand_scope(&doc, &fragment);
    let tx_update = {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
        txn.commit();
        txn.encode_update_v1()
    };
    assert!(tx_update.len() <= compiled.encoded_growth_bound);
    let after_full_len = doc
        .transact()
        .encode_state_as_update_v1(&StateVector::default())
        .len();
    assert!(after_full_len <= before_full_len + compiled.encoded_growth_bound);
    let item = undo.undo_stack().last().unwrap();
    let actual = item
        .deletions()
        .iter()
        .flat_map(|(_, ranges)| ranges.into_iter())
        .map(|range| u64::from(range.end - range.start))
        .sum::<u64>();
    assert!(actual > 0, "formatting should clean up hidden CRDT items");
    assert!(actual <= compiled.undo_units_bound);
}

#[test]
fn join_blocks_directly_keeps_the_left_block_and_text() {
    let source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "ab" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "cd" }] }
        ]
    });
    let (actual, expected, _, update_len, estimate) = compile_and_execute(
        source,
        vec![TypedOperation::JoinBlocks {
            at: point_for_test(2),
        }],
    );
    assert_eq!(actual, expected);
    assert_eq!(actual["content"].as_array().unwrap().len(), 1);
    assert_eq!(actual["content"][0]["content"][0]["text"], "abcd");
    assert!(update_len <= estimate);
}

#[test]
fn join_blocks_folds_edits_on_both_sides_and_targets_the_retained_text_afterward() {
    let source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "ab" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "cd" }] }
        ]
    });
    let (right, expected_right, _, _, _) = compile_and_execute(
        source.clone(),
        vec![
            TypedOperation::InsertText {
                at: point_for_test(4),
                text: "R".into(),
                marks: vec![],
            },
            TypedOperation::JoinBlocks {
                at: point_for_test(2),
            },
        ],
    );
    assert_eq!(right, expected_right);
    assert_eq!(right["content"][0]["content"][0]["text"], "abcRd");

    let (left, expected_left, _, _, _) = compile_and_execute(
        source.clone(),
        vec![
            TypedOperation::InsertText {
                at: point_for_test(1),
                text: "L".into(),
                marks: vec![],
            },
            TypedOperation::JoinBlocks {
                at: point_for_test(2),
            },
        ],
    );
    assert_eq!(left, expected_left);
    assert_eq!(left["content"][0]["content"][0]["text"], "aLbcd");

    let (after, expected_after, _, _, _) = compile_and_execute(
        source,
        vec![
            TypedOperation::JoinBlocks {
                at: point_for_test(2),
            },
            TypedOperation::InsertText {
                at: point_for_test(2),
                text: "X".into(),
                marks: vec![],
            },
            TypedOperation::AddMark {
                range: range_for_test(2, 4),
                mark: Mark::new("bold".into(), HashMap::new()),
            },
        ],
    );
    assert_eq!(after, expected_after);
    let pieces = after["content"][0]["content"].as_array().unwrap();
    assert_eq!(
        pieces
            .iter()
            .filter_map(|node| node["text"].as_str())
            .collect::<String>(),
        "abXcd"
    );
    assert!(pieces.iter().any(|node| {
        node["marks"]
            .as_array()
            .is_some_and(|marks| marks.iter().any(|mark| mark["type"] == "bold"))
    }));
}

#[test]
fn join_blocks_preserves_left_identity_marks_sticky_and_accepts_follow_up_attrs() {
    let source = json!({
        "type": "doc",
        "content": [
            { "type": "h2", "attrs": { "id": "left" }, "content": [{ "type": "text", "text": "ab" }] },
            { "type": "h2", "attrs": { "id": "right" }, "content": [{ "type": "text", "text": "😀c", "marks": [{ "type": "bold" }] }] },
            { "type": "h2", "attrs": { "id": "tail" }, "content": [{ "type": "text", "text": "tail" }] }
        ]
    });
    let (doc, schema, limits, mut compiled) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::JoinBlocks {
                at: point_for_test(2),
            },
            TypedOperation::UpdateNodeAttrs {
                at: point_for_test(0),
                attrs: HashMap::from([("id".into(), Value::String("joined".into()))]),
            },
        ],
        attribute_schema(),
    );
    assert!(matches!(
        compiled.mutation_plan.actions.as_slice(),
        [
            YrsMutationAction::InsertText { .. },
            YrsMutationAction::DeleteXmlChildren {
                child_index: 1,
                child_count: 1,
                ..
            },
            YrsMutationAction::SetXmlAttribute { .. }
        ]
    ));
    let (left_block_id, left_text_id, tail_block_id, tail_text_id, sticky) = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let children = fragment.children(&txn).collect::<Vec<_>>();
        let left_text = paragraph_text(&fragment, &txn, 0);
        let tail_text = paragraph_text(&fragment, &txn, 2);
        let sticky = StickyIndex::at(
            &txn,
            BranchPtr::from(<XmlTextRef as AsRef<Branch>>::as_ref(&tail_text)),
            2,
            Assoc::After,
        )
        .unwrap();
        (
            children[0].id(),
            <XmlTextRef as AsRef<Branch>>::as_ref(&left_text).id(),
            children[2].id(),
            <XmlTextRef as AsRef<Branch>>::as_ref(&tail_text).id(),
            sticky,
        )
    };
    {
        let txn = doc.transact();
        let preflight =
            preflight_mutation_work_for_test(122, &compiled.mutation_plan, &txn).unwrap();
        let exact = compiled.mutation_plan.compilation_work_for_test() + preflight;
        compiled.mutation_plan.set_work_limit_for_test(exact);
        preflight_mutation_plan(122, &compiled.mutation_plan, &txn).unwrap();
        compiled.mutation_plan.set_work_limit_for_test(exact - 1);
        assert_eq!(
            preflight_mutation_plan(122, &compiled.mutation_plan, &txn)
                .unwrap_err()
                .code,
            "OPERATION_LIMIT_EXCEEDED"
        );
        compiled.mutation_plan.set_work_limit_for_test(exact);
    }
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let children = fragment.children(&txn).collect::<Vec<_>>();
    assert_eq!(children[0].id(), left_block_id);
    assert_eq!(children[1].id(), tail_block_id);
    assert_eq!(
        <XmlTextRef as AsRef<Branch>>::as_ref(&paragraph_text(&fragment, &txn, 0)).id(),
        left_text_id
    );
    assert_eq!(
        <XmlTextRef as AsRef<Branch>>::as_ref(&paragraph_text(&fragment, &txn, 1)).id(),
        tail_text_id
    );
    assert_eq!(sticky.get_offset(&txn).unwrap().branch.id(), tail_text_id);
    let actual = YrsDocumentCodec::new(&schema, &limits)
        .read_json(&fragment, &txn)
        .unwrap();
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][0]["attrs"]["id"], "joined");
    assert_eq!(actual["content"][0]["content"][1]["text"], "😀c");
    assert_eq!(
        actual["content"][0]["content"][1]["marks"][0]["type"],
        "bold"
    );
}

#[test]
fn join_blocks_disambiguates_equal_neighbors_and_ascends_to_nested_siblings() {
    let equal = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "xx" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "xx" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "xx" }] }
        ]
    });
    let (doc, _, _, compiled) = compile_operations_with_schema(
        &equal,
        vec![TypedOperation::JoinBlocks {
            at: point_for_test(5),
        }],
        tiptap_schema(),
    );
    let before_ids = {
        let txn = doc.transact();
        txn.get_xml_fragment("prosemirror")
            .unwrap()
            .children(&txn)
            .map(|child| child.id())
            .collect::<Vec<_>>()
    };
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let after_ids = fragment
        .children(&txn)
        .map(|child| child.id())
        .collect::<Vec<_>>();
    assert_eq!(
        after_ids,
        vec![before_ids[0].clone(), before_ids[1].clone()]
    );

    let nested = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "ab" }] }] },
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "cd" }] }] }
            ]
        }]
    });
    let schema = tiptap_schema();
    let nested_document =
        from_prosemirror_json(&nested, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&nested_document, &schema);
    let nested_byte = rendered.find("ab").unwrap();
    let nested_offset =
        u32::try_from(rendered[..nested_byte].chars().count().saturating_add(2)).unwrap();
    let (actual, expected, _, _, _) = compile_and_execute(
        nested,
        vec![TypedOperation::JoinBlocks {
            at: point_for_test(nested_offset),
        }],
    );
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][0]["content"].as_array().unwrap().len(), 1);
    assert_eq!(
        actual["content"][0]["content"][0]["content"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn join_blocks_handles_empty_sides_and_keeps_the_first_block_type_and_attrs() {
    let empty_left = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph" },
            { "type": "paragraph", "content": [{ "type": "text", "text": "right" }] }
        ]
    });
    let (actual, expected, _, _, _) = compile_and_execute(
        empty_left,
        vec![TypedOperation::JoinBlocks {
            at: point_for_test(0),
        }],
    );
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][0]["content"][0]["text"], "right");

    let empty_right = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "left" }] },
            { "type": "paragraph" }
        ]
    });
    let (actual, expected, _, _, _) = compile_and_execute(
        empty_right,
        vec![TypedOperation::JoinBlocks {
            at: point_for_test(4),
        }],
    );
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][0]["content"][0]["text"], "left");

    let differing = json!({
        "type": "doc",
        "content": [
            { "type": "h2", "attrs": { "id": "first" }, "content": [{ "type": "text", "text": "a" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "b" }] }
        ]
    });
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &differing,
        vec![TypedOperation::JoinBlocks {
            at: point_for_test(1),
        }],
        attribute_schema(),
    );
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let actual = YrsDocumentCodec::new(&schema, &limits)
        .read_json(&fragment, &txn)
        .unwrap();
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][0]["type"], "h2");
    assert_eq!(actual["content"][0]["attrs"]["id"], "first");
}

#[test]
fn join_blocks_copies_mixed_right_children_into_the_retained_left_element() {
    let source = json!({
        "type": "doc",
        "content": [
            {
                "type": "paragraph",
                "content": [
                    { "type": "text", "text": "L" },
                    { "type": "hardBreak" }
                ]
            },
            {
                "type": "paragraph",
                "content": [
                    { "type": "text", "text": "R", "marks": [{ "type": "bold" }] },
                    { "type": "hardBreak" },
                    { "type": "text", "text": "T" }
                ]
            }
        ]
    });
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![TypedOperation::JoinBlocks {
            at: point_for_test(2),
        }],
        tiptap_schema(),
    );
    let (left_block_id, left_child_ids, right_child_ids, before_full_len) = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let XmlOut::Element(left) = fragment.get(&txn, 0).unwrap() else {
            panic!("left paragraph expected")
        };
        let XmlOut::Element(right) = fragment.get(&txn, 1).unwrap() else {
            panic!("right paragraph expected")
        };
        (
            <yrs::types::xml::XmlElementRef as AsRef<Branch>>::as_ref(&left).id(),
            left.children(&txn)
                .map(|child| child.id())
                .collect::<Vec<_>>(),
            right
                .children(&txn)
                .map(|child| child.id())
                .collect::<Vec<_>>(),
            txn.encode_state_as_update_v1(&StateVector::default()).len(),
        )
    };
    assert!(matches!(
        compiled.mutation_plan.actions.as_slice(),
        [
            YrsMutationAction::InsertXmlChildren { .. },
            YrsMutationAction::DeleteXmlChildren { .. }
        ]
    ));
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    let estimate = compiled.encoded_growth_bound;
    let update = {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
        txn.commit();
        txn.encode_update_v1()
    };
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let XmlOut::Element(left) = fragment.get(&txn, 0).unwrap() else {
        panic!("retained paragraph expected")
    };
    assert_eq!(
        <yrs::types::xml::XmlElementRef as AsRef<Branch>>::as_ref(&left).id(),
        left_block_id
    );
    let after_child_ids = left
        .children(&txn)
        .map(|child| child.id())
        .collect::<Vec<_>>();
    assert_eq!(&after_child_ids[..left_child_ids.len()], left_child_ids);
    assert!(after_child_ids[left_child_ids.len()..]
        .iter()
        .all(|id| !right_child_ids.contains(id)));
    let actual = YrsDocumentCodec::new(&schema, &limits)
        .read_json(&fragment, &txn)
        .unwrap();
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][0]["content"][2]["text"], "R");
    assert_eq!(
        actual["content"][0]["content"][2]["marks"][0]["type"],
        "bold"
    );
    assert!(update.len() <= estimate);
    let after_full_len = txn.encode_state_as_update_v1(&StateVector::default()).len();
    assert!(after_full_len <= before_full_len + estimate);
}

#[test]
fn join_blocks_copies_nested_any_attributes_without_flattening() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "taskList",
            "content": [
                {
                    "type": "taskItem",
                    "attrs": { "checked": true },
                    "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "a" }] }]
                },
                {
                    "type": "taskItem",
                    "attrs": { "checked": false },
                    "content": [
                        { "type": "paragraph", "content": [{ "type": "text", "text": "b" }] },
                        { "type": "customBlock", "attrs": { "meta": { "nested": [1, false, "x"] } } }
                    ]
                }
            ]
        }]
    });
    let schema = attribute_schema();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&document, &schema);
    let byte = rendered.find('a').unwrap();
    let offset = u32::try_from(rendered[..byte].chars().count() + 1).unwrap();
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![TypedOperation::JoinBlocks {
            at: point_for_test(offset),
        }],
        schema,
    );
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let actual = YrsDocumentCodec::new(&schema, &limits)
        .read_json(&fragment, &txn)
        .unwrap();
    assert_eq!(actual, expected);
    assert_eq!(
        actual["content"][0]["content"][0]["content"][2]["attrs"]["meta"],
        json!({ "nested": [1, false, "x"] })
    );
}

#[test]
fn structural_replace_trims_marked_unicode_text_endpoints_around_an_atom() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [
                { "type": "text", "text": "A😀X" },
                { "type": "hardBreak" },
                { "type": "text", "text": "e\u{301}ZY" }
            ]
        }]
    });
    let (doc, schema, limits, editing_limits, _) = diagnostic_doc(&source);
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let XmlOut::Element(paragraph) = fragment.get(&txn, 0).unwrap() else {
            panic!("expected paragraph")
        };
        let XmlOut::Text(left) = paragraph.get(&txn, 0).unwrap() else {
            panic!("expected left text")
        };
        let XmlOut::Text(right) = paragraph.get(&txn, 2).unwrap() else {
            panic!("expected right text")
        };
        left.format(
            &mut txn,
            1,
            3,
            Attrs::from([(Arc::<str>::from("bold"), Any::Bool(true))]),
        );
        right.format(
            &mut txn,
            0,
            2,
            Attrs::from([(Arc::<str>::from("italic"), Any::Bool(true))]),
        );
    }
    let codec = YrsDocumentCodec::new(&schema, &limits);
    let (document, left_id, old_atom_id, right_id, before_full_len) = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let XmlOut::Element(paragraph) = fragment.get(&txn, 0).unwrap() else {
            panic!("expected paragraph")
        };
        let children = paragraph.children(&txn).collect::<Vec<_>>();
        let json = codec.read_json(&fragment, &txn).unwrap();
        (
            from_prosemirror_json(&json, &schema, UnknownTypeMode::Preserve).unwrap(),
            children[0].id(),
            children[1].id(),
            children[2].id(),
            txn.encode_state_as_update_v1(&StateVector::default()).len(),
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
                request_id: 114,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::ReplaceRange {
                    range: range_for_test(2, 6),
                    content: Fragment::from(vec![Node::void("hardBreak".into(), HashMap::new())]),
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
        compiled.mutation_plan.actions.as_slice(),
        [
            YrsMutationAction::DeleteText {
                index_utf16: 0,
                len_utf16: 2,
                ..
            },
            YrsMutationAction::DeleteText {
                index_utf16: 3,
                len_utf16: 1,
                ..
            },
            YrsMutationAction::DeleteXmlChildren {
                child_index: 1,
                child_count: 1,
                ..
            },
            YrsMutationAction::InsertXmlChildren { child_index: 1, .. }
        ]
    ));
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    let estimate = compiled.encoded_growth_bound;
    let update = {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
        txn.commit();
        txn.encode_update_v1()
    };
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let XmlOut::Element(paragraph) = fragment.get(&txn, 0).unwrap() else {
        panic!("expected paragraph")
    };
    let children = paragraph.children(&txn).collect::<Vec<_>>();
    assert_eq!(children.len(), 3);
    assert_eq!(children[0].id(), left_id);
    assert_ne!(children[1].id(), old_atom_id);
    assert_eq!(children[2].id(), right_id);
    assert_eq!(codec.read_json(&fragment, &txn).unwrap(), expected);
    assert_eq!(expected["content"][0]["content"][0]["text"], "A");
    assert_eq!(expected["content"][0]["content"][1]["text"], "😀");
    assert_eq!(
        expected["content"][0]["content"][1]["marks"][0]["type"],
        "bold"
    );
    assert_eq!(expected["content"][0]["content"][3]["text"], "ZY");
    assert!(update.len() <= estimate);
    let after_full_len = txn.encode_state_as_update_v1(&StateVector::default()).len();
    assert!(after_full_len <= before_full_len + estimate);
}

#[test]
fn structural_replace_disambiguates_duplicate_equal_atoms_by_position() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "hardBreak" }, { "type": "hardBreak" }]
        }]
    });
    let replace_and_ids = |from, to| {
        let (doc, schema, limits, editing_limits, document) = diagnostic_doc(&source);
        let original_ids = {
            let txn = doc.transact();
            let fragment = txn.get_xml_fragment("prosemirror").unwrap();
            let XmlOut::Element(paragraph) = fragment.get(&txn, 0).unwrap() else {
                panic!("expected paragraph")
            };
            paragraph
                .children(&txn)
                .map(|child| child.id())
                .collect::<Vec<_>>()
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
                    request_id: 115,
                    base_document_revision: 0,
                    origin: TransactionOrigin::LocalInput,
                    operations: vec![TypedOperation::ReplaceRange {
                        range: range_for_test(from, to),
                        content: Fragment::from(vec![Node::text("x".into(), vec![])]),
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
            execute_mutation_plan(compiled.mutation_plan, &mut txn);
        }
        let after_ids = {
            let txn = doc.transact();
            let fragment = txn.get_xml_fragment("prosemirror").unwrap();
            let XmlOut::Element(paragraph) = fragment.get(&txn, 0).unwrap() else {
                panic!("expected paragraph")
            };
            paragraph
                .children(&txn)
                .map(|child| child.id())
                .collect::<Vec<_>>()
        };
        (original_ids, after_ids)
    };

    let (before_first, after_first) = replace_and_ids(0, 1);
    assert_ne!(after_first[0], before_first[0]);
    assert_eq!(after_first[1], before_first[1]);
    let (before_second, after_second) = replace_and_ids(1, 2);
    assert_eq!(after_second[0], before_second[0]);
    assert_ne!(after_second[1], before_second[1]);
}

#[test]
fn structural_delete_maps_mark_runs_to_storage_and_preserves_unaffected_sticky() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [
                { "type": "text", "text": "ab" },
                { "type": "hardBreak" },
                { "type": "text", "text": "cd" }
            ]
        }]
    });
    let (doc, schema, limits, editing_limits, _) = diagnostic_doc(&source);
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let first = paragraph_text(&fragment, &txn, 0);
        first.format(
            &mut txn,
            1,
            1,
            Attrs::from([(Arc::<str>::from("bold"), Any::Bool(true))]),
        );
    }
    let codec = YrsDocumentCodec::new(&schema, &limits);
    let (document, left_id, right_id, sticky, before_full_len) = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let json = codec.read_json(&fragment, &txn).unwrap();
        let document = from_prosemirror_json(&json, &schema, UnknownTypeMode::Preserve).unwrap();
        let XmlOut::Element(paragraph) = fragment.get(&txn, 0).unwrap() else {
            panic!("expected paragraph")
        };
        let children = paragraph.children(&txn).collect::<Vec<_>>();
        let left_id = children[0].id();
        let right_id = children[2].id();
        let XmlOut::Text(right) = &children[2] else {
            panic!("expected right text")
        };
        let sticky = StickyIndex::at(
            &txn,
            BranchPtr::from(<XmlTextRef as AsRef<Branch>>::as_ref(right)),
            1,
            Assoc::After,
        )
        .unwrap();
        (
            document,
            left_id,
            right_id,
            sticky,
            txn.encode_state_as_update_v1(&StateVector::default()).len(),
        )
    };
    let mut compiled = {
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
                request_id: 110,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::DeleteRange {
                    range: range_for_test(2, 3),
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
        let txn = doc.transact();
        let preflight =
            preflight_mutation_work_for_test(110, &compiled.mutation_plan, &txn).unwrap();
        let exact = compiled.mutation_plan.compilation_work_for_test() + preflight;
        compiled.mutation_plan.set_work_limit_for_test(exact);
        preflight_mutation_plan(110, &compiled.mutation_plan, &txn).unwrap();
        compiled.mutation_plan.set_work_limit_for_test(exact - 1);
        assert_eq!(
            preflight_mutation_plan(110, &compiled.mutation_plan, &txn)
                .unwrap_err()
                .code,
            "OPERATION_LIMIT_EXCEEDED"
        );
        compiled.mutation_plan.set_work_limit_for_test(exact);
    }
    let estimate = compiled.encoded_growth_bound;
    let update = {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
        txn.commit();
        txn.encode_update_v1()
    };
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let XmlOut::Element(paragraph) = fragment.get(&txn, 0).unwrap() else {
        panic!("expected paragraph")
    };
    let children = paragraph.children(&txn).collect::<Vec<_>>();
    assert_eq!(children.len(), 2);
    assert_eq!(children[0].id(), left_id);
    assert_eq!(children[1].id(), right_id);
    assert_eq!(sticky.get_offset(&txn).unwrap().index, 1);
    let actual = codec.read_json(&fragment, &txn).unwrap();
    assert_eq!(actual, to_prosemirror_json(&compiled.preview, &schema));
    assert!(update.len() <= estimate, "{} > {estimate}", update.len());
    let after_full_len = txn.encode_state_as_update_v1(&StateVector::default()).len();
    assert!(after_full_len <= before_full_len + estimate);
}

#[test]
fn structural_delete_preflight_rejects_same_count_child_substitution_atomically() {
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
                request_id: 111,
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
        .unwrap()
    };
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let XmlOut::Element(paragraph) = fragment.get(&txn, 0).unwrap() else {
            panic!("expected paragraph")
        };
        paragraph.remove_range(&mut txn, 0, 1);
        paragraph.insert(&mut txn, 0, XmlElementPrelim::empty("hardBreak"));
    }
    let txn = doc.transact();
    let before = txn.state_vector();
    let error = preflight_mutation_plan(111, &compiled.mutation_plan, &txn).unwrap_err();
    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(txn.state_vector(), before);
}

#[test]
fn pure_insert_preflight_rejects_same_count_parent_substitution_atomically() {
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
                request_id: 118,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalCommand,
                operations: vec![TypedOperation::InsertNode {
                    at: point_for_test(0),
                    node: Node::void("hardBreak".into(), HashMap::new()),
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
        compiled.mutation_plan.actions.as_slice(),
        [YrsMutationAction::InsertXmlChildren { .. }]
    ));
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        fragment.remove_range(&mut txn, 0, 1);
        let paragraph = fragment.insert(&mut txn, 0, XmlElementPrelim::empty("paragraph"));
        paragraph.insert(&mut txn, 0, XmlElementPrelim::empty("hardBreak"));
    }
    let txn = doc.transact();
    let before = txn.encode_state_as_update_v1(&StateVector::default());
    let error = preflight_mutation_plan(118, &compiled.mutation_plan, &txn).unwrap_err();
    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(
        txn.encode_state_as_update_v1(&StateVector::default()),
        before
    );
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    assert_eq!(
        YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap(),
        source
    );
}

#[test]
fn nested_text_and_attribute_preflight_reject_gc_replaced_parents_without_panicking() {
    let text_source = json!({
        "type": "doc",
        "content": [{
            "type": "blockquote",
            "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "abc" }] }]
        }]
    });
    let (text_doc, schema, limits, editing_limits, document) = diagnostic_doc(&text_source);
    let mut text_compiled = {
        let txn = text_doc.transact();
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
                request_id: 119,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalCommand,
                operations: vec![TypedOperation::InsertText {
                    at: point_for_test(1),
                    text: "X".into(),
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
    let text_exact = text_compiled.mutation_plan.compilation_work_for_test()
        + text_compiled
            .mutation_plan
            .expected_preflight_work_for_test();
    {
        let mut txn = text_doc.transact_mut();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        fragment.remove_range(&mut txn, 0, 1);
        let quote = fragment.insert(&mut txn, 0, XmlElementPrelim::empty("blockquote"));
        let paragraph = quote.insert(&mut txn, 0, XmlElementPrelim::empty("paragraph"));
        paragraph.insert(&mut txn, 0, XmlTextPrelim::new("abc"));
    }
    let txn = text_doc.transact();
    let before = txn.encode_state_as_update_v1(&StateVector::default());
    text_compiled
        .mutation_plan
        .set_work_limit_for_test(text_exact - 1);
    assert_eq!(
        preflight_mutation_plan(119, &text_compiled.mutation_plan, &txn)
            .unwrap_err()
            .code,
        "OPERATION_LIMIT_EXCEEDED"
    );
    text_compiled
        .mutation_plan
        .set_work_limit_for_test(text_exact);
    assert_eq!(
        preflight_mutation_plan(119, &text_compiled.mutation_plan, &txn)
            .unwrap_err()
            .code,
        "ENGINE_INVARIANT_FAILED"
    );
    assert_eq!(
        txn.encode_state_as_update_v1(&StateVector::default()),
        before
    );

    let attr_source = json!({
        "type": "doc",
        "content": [{
            "type": "blockquote",
            "content": [{
                "type": "h2",
                "attrs": { "id": "old" },
                "content": [{ "type": "text", "text": "heading" }]
            }]
        }]
    });
    let (attr_doc, attr_schema, attr_limits, attr_editing, attr_document) =
        diagnostic_doc_with_schema(&attr_source, attribute_schema());
    let attr_compiled = {
        let txn = attr_doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        compile_transaction_with_yrs(
            CompilationContext {
                document: &attr_document,
                selection: None,
                schema: &attr_schema,
                resource_limits: &attr_limits,
                editing_limits: &attr_editing,
                document_revision: 0,
                max_length: None,
            },
            TypedTransaction {
                request_id: 120,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalCommand,
                operations: vec![TypedOperation::UpdateNodeAttrs {
                    at: point_for_test(1),
                    attrs: HashMap::from([("id".into(), Value::String("new".into()))]),
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
        let mut txn = attr_doc.transact_mut();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        fragment.remove_range(&mut txn, 0, 1);
        let quote = fragment.insert(&mut txn, 0, XmlElementPrelim::empty("blockquote"));
        let heading = quote.insert(&mut txn, 0, XmlElementPrelim::empty("heading"));
        heading.insert_attribute(&mut txn, "level", Any::BigInt(2));
        heading.insert_attribute(&mut txn, "id", Any::String("old".into()));
        heading.insert(&mut txn, 0, XmlTextPrelim::new("heading"));
    }
    let txn = attr_doc.transact();
    let before = txn.encode_state_as_update_v1(&StateVector::default());
    assert_eq!(
        preflight_mutation_plan(120, &attr_compiled.mutation_plan, &txn)
            .unwrap_err()
            .code,
        "ENGINE_INVARIANT_FAILED"
    );
    assert_eq!(
        txn.encode_state_as_update_v1(&StateVector::default()),
        before
    );
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
    let (sibling_id, sticky, before_full_len) = {
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
        (
            id,
            sticky,
            txn.encode_state_as_update_v1(&StateVector::default()).len(),
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
    let update = {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
        txn.commit();
        txn.encode_update_v1()
    };
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
    assert!(update.len() <= estimate);
    let after_full_len = txn.encode_state_as_update_v1(&StateVector::default()).len();
    assert!(after_full_len <= before_full_len + estimate);
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
    // This is the exact admitted input plus reserved and reconciled Yrs
    // materialization, coordinate-index, and clock traversal work.
    compile(51, &source, vec![insertion()]).unwrap();
    let one_over = compile(50, &source, vec![insertion()]).unwrap_err();
    assert_eq!(one_over.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(one_over.limit, Some(50));

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
    assert!(compile(16_442, &large_bold, noops.clone())
        .unwrap()
        .mutation_plan
        .actions
        .is_empty());
    assert_eq!(
        compile(16_441, &large_bold, noops).unwrap_err().limit,
        Some(16_441)
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
    // 6 admitted mark bytes + 12 bytes for materialization/coordinate indexing
    // + 22 reserved Yrs clock units across the two pre-lowering traversals.
    exact_limits.max_input_bytes = 40;
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
    exact_limits.max_input_bytes = 39;
    assert_eq!(compile_noop(&exact_limits).unwrap_err().limit, Some(39));
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
    // Exact normalized text/mark signatures add one charged run and content
    // comparison unit per target while preserving linear traversal.
    assert!(traversal_work <= BLOCKS * 20, "{traversal_work}");

    let boundary_count = BLOCKS * 3 + 1;
    let boundaries = (0..u32::try_from(boundary_count).unwrap()).collect::<Vec<_>>();
    let before = compiler.total_mutation_work_for_test();
    let disposition = compiler
        .delete(0, 1, u32::try_from(BLOCKS * 3 - 1).unwrap(), &boundaries)
        .unwrap();
    assert_eq!(disposition, TextRangeDisposition::Structural);
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
    let (unaffected_id, sticky, before_full_len) = {
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
        (
            id,
            sticky,
            txn.encode_state_as_update_v1(&StateVector::default()).len(),
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
    let update = {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
        txn.commit();
        txn.encode_update_v1()
    };
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
    assert!(!update.is_empty());
    assert!(update.len() <= estimate);
    assert!(update.len() < TWO_PARAGRAPHS.len());
    let after_full_len = txn.encode_state_as_update_v1(&StateVector::default()).len();
    assert!(after_full_len <= before_full_len + estimate);
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
fn document_guard_rejects_deletion_only_staleness_with_an_unchanged_state_vector() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "Hello" }]
        }]
    });
    let (doc, _schema, _limits, compiled) = compile_operations_with_schema(
        &source,
        vec![TypedOperation::InsertText {
            at: point_for_test(1),
            text: "x".into(),
            marks: vec![],
        }],
        tiptap_schema(),
    );
    let before_delete_vector = doc.transact().state_vector();
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        paragraph_text(&fragment, &txn, 0).remove_range(&mut txn, 0, 1);
    }
    let txn = doc.transact();
    assert_eq!(txn.state_vector(), before_delete_vector);
    let rejected_state = txn.encode_state_as_update_v1(&StateVector::default());
    let error = preflight_mutation_plan(175, &compiled.mutation_plan, &txn).unwrap_err();
    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(error.operation_index, Some(0));
    assert_eq!(
        txn.encode_state_as_update_v1(&StateVector::default()),
        rejected_state
    );
}

#[test]
fn document_guard_rejects_a_foreign_same_content_yrs_store_without_mutation() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "Hello" }]
        }]
    });
    let (_source_doc, _schema, _limits, compiled) = compile_operations_with_schema(
        &source,
        vec![TypedOperation::InsertText {
            at: point_for_test(1),
            text: "x".into(),
            marks: vec![],
        }],
        tiptap_schema(),
    );
    let (foreign, _, _, _, _) = diagnostic_doc(&source);
    let txn = foreign.transact();
    let before = txn.encode_state_as_update_v1(&StateVector::default());
    let error = preflight_mutation_plan(176, &compiled.mutation_plan, &txn).unwrap_err();
    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(error.operation_index, Some(0));
    assert_eq!(
        txn.encode_state_as_update_v1(&StateVector::default()),
        before
    );
}

#[test]
fn document_guard_rejects_hostile_stale_attributes_before_live_attribute_materialization() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "blockquote",
            "content": [{
                "type": "h2",
                "attrs": { "id": "old" },
                "content": [{ "type": "text", "text": "heading" }]
            }]
        }]
    });
    let (doc, schema, limits, editing_limits, document) =
        diagnostic_doc_with_schema(&source, attribute_schema());
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
                request_id: 177,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalCommand,
                operations: vec![TypedOperation::UpdateNodeAttrs {
                    at: point_for_test(1),
                    attrs: HashMap::from([("id".into(), Value::String("new".into()))]),
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
        let XmlOut::Element(quote) = fragment.get(&txn, 0).unwrap() else {
            panic!("expected blockquote")
        };
        let XmlOut::Element(heading) = quote.get(&txn, 0).unwrap() else {
            panic!("expected heading")
        };
        heading.insert_attribute(
            &mut txn,
            "hostile",
            Any::String("x".repeat(1024 * 1024).into()),
        );
    }
    let txn = doc.transact();
    let before = txn.encode_state_as_update_v1(&StateVector::default());
    let error = preflight_mutation_plan(177, &compiled.mutation_plan, &txn).unwrap_err();
    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(error.operation_index, Some(0));
    assert_eq!(
        txn.encode_state_as_update_v1(&StateVector::default()),
        before
    );
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

fn literal_list_item_attr_schema() -> crate::schema::Schema {
    crate::schema::Schema::from_json(&json!({
        "nodes": [
            { "name": "doc", "content": "block+", "role": "doc" },
            { "name": "paragraph", "content": "inline*", "group": "block", "role": "textBlock", "htmlTag": "p" },
            { "name": "hardBreak", "content": "", "group": "inline", "role": "hardBreak", "htmlTag": "br", "isVoid": true },
            { "name": "taskList", "content": "listItem+", "group": "block", "role": "list", "htmlTag": "ul", "attrs": { "listMeta": { "default": null } } },
            { "name": "listItem", "content": "paragraph block*", "role": "listItem", "htmlTag": "li", "attrs": { "checked": { "default": null }, "itemMeta": { "default": null } } },
            { "name": "text", "group": "inline", "role": "text" }
        ],
        "marks": []
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

fn compile_and_execute_attribute_update(
    source: Value,
    attrs: HashMap<String, Value>,
) -> (Value, Value) {
    let (doc, schema, limits, editing_limits, document) =
        diagnostic_doc_with_schema(&source, attribute_schema());
    let (before_ids, before_full_len, sticky) = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let sticky = first_xml_text(&fragment, &txn).and_then(|text| {
            StickyIndex::at(
                &txn,
                BranchPtr::from(<XmlTextRef as AsRef<Branch>>::as_ref(&text)),
                0,
                Assoc::After,
            )
        });
        (
            collect_xml_ids(&fragment, &txn),
            txn.encode_state_as_update_v1(&StateVector::default()).len(),
            sticky,
        )
    };
    let mut compiled = {
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
                request_id: 117,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::UpdateNodeAttrs {
                    at: point_for_test(0),
                    attrs,
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
        .unwrap()
    };
    let keys = compiled
        .mutation_plan
        .actions
        .iter()
        .filter_map(|action| match action {
            YrsMutationAction::SetXmlAttribute { key, .. }
            | YrsMutationAction::RemoveXmlAttribute { key, .. } => Some(key.as_ref()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(keys.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(keys.len(), compiled.mutation_plan.actions.len());
    {
        let txn = doc.transact();
        let preflight =
            preflight_mutation_work_for_test(117, &compiled.mutation_plan, &txn).unwrap();
        let exact = compiled.mutation_plan.compilation_work_for_test() + preflight;
        compiled.mutation_plan.set_work_limit_for_test(exact);
        preflight_mutation_plan(117, &compiled.mutation_plan, &txn).unwrap();
        compiled.mutation_plan.set_work_limit_for_test(exact - 1);
        assert_eq!(
            preflight_mutation_plan(117, &compiled.mutation_plan, &txn)
                .unwrap_err()
                .code,
            "OPERATION_LIMIT_EXCEEDED"
        );
        compiled.mutation_plan.set_work_limit_for_test(exact);
    }
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    let estimate = compiled.encoded_growth_bound;
    let has_actions = !compiled.mutation_plan.actions.is_empty();
    let update = if has_actions {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
        txn.commit();
        txn.encode_update_v1()
    } else {
        Vec::new()
    };
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    assert_eq!(collect_xml_ids(&fragment, &txn), before_ids);
    if let Some(sticky) = sticky {
        assert!(sticky.get_offset(&txn).is_some());
    }
    let update_len = update.len();
    assert!(update_len <= estimate, "{update_len} > {estimate}");
    let after_full_len = txn.encode_state_as_update_v1(&StateVector::default()).len();
    assert!(after_full_len <= before_full_len + estimate);
    (
        YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap(),
        expected,
    )
}

fn compile_attribute_operations(
    source: Value,
    updates: Vec<HashMap<String, Value>>,
) -> (
    Doc,
    crate::schema::Schema,
    ResourceLimits,
    CompiledTransaction,
) {
    let (doc, schema, limits, editing_limits, document) =
        diagnostic_doc_with_schema(&source, attribute_schema());
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
                request_id: 118,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalInput,
                operations: updates
                    .into_iter()
                    .map(|attrs| TypedOperation::UpdateNodeAttrs {
                        at: point_for_test(0),
                        attrs,
                    })
                    .collect(),
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

fn collect_xml_ids<T: ReadTxn>(
    fragment: &yrs::types::xml::XmlFragmentRef,
    txn: &T,
) -> Vec<yrs::branch::BranchID> {
    fn visit<T: ReadTxn>(out: XmlOut, txn: &T, ids: &mut Vec<yrs::branch::BranchID>) {
        ids.push(out.id());
        match out {
            XmlOut::Element(element) => {
                for child in element.children(txn) {
                    visit(child, txn, ids);
                }
            }
            XmlOut::Fragment(fragment) => {
                for child in fragment.children(txn) {
                    visit(child, txn, ids);
                }
            }
            XmlOut::Text(_) => {}
        }
    }
    let mut ids = Vec::new();
    for child in fragment.children(txn) {
        visit(child, txn, &mut ids);
    }
    ids
}

fn first_xml_text<T: ReadTxn>(
    fragment: &yrs::types::xml::XmlFragmentRef,
    txn: &T,
) -> Option<XmlTextRef> {
    fn visit<T: ReadTxn>(out: XmlOut, txn: &T) -> Option<XmlTextRef> {
        match out {
            XmlOut::Text(text) => Some(text),
            XmlOut::Element(element) => element.children(txn).find_map(|child| visit(child, txn)),
            XmlOut::Fragment(fragment) => {
                fragment.children(txn).find_map(|child| visit(child, txn))
            }
        }
    }
    fragment.children(txn).find_map(|child| visit(child, txn))
}

#[test]
fn generated_structural_trees_bound_and_converge_for_256_fixed_seeds() {
    fn nested_doc(mut blocks: Vec<Value>, depth: usize) -> Value {
        for _ in 0..depth {
            blocks = vec![json!({ "type": "blockquote", "content": blocks })];
        }
        json!({ "type": "doc", "content": blocks })
    }

    let schema = tiptap_schema();
    for seed in 0usize..256 {
        let depth = (seed / 11) % 4;
        let (source, operations) = match seed % 11 {
            0 => {
                let source = nested_doc(
                    vec![
                        json!({ "type": "paragraph", "content": [{ "type": "text", "text": "A😀B" }] }),
                    ],
                    depth,
                );
                (
                    source,
                    vec![TypedOperation::InsertNode {
                        at: point_for_test(2),
                        node: Node::void("hardBreak".into(), HashMap::new()),
                    }],
                )
            }
            1 => {
                let source = nested_doc(
                    vec![
                        json!({ "type": "paragraph", "content": [{ "type": "text", "text": "A😀B" }] }),
                    ],
                    0,
                );
                (
                    source,
                    vec![TypedOperation::SplitBlock {
                        at: point_for_test(2),
                        node_type: "paragraph".into(),
                        attrs: HashMap::new(),
                    }],
                )
            }
            2 => {
                let source = nested_doc(
                    vec![
                        json!({ "type": "paragraph", "content": [{ "type": "text", "text": "ab" }] }),
                        json!({ "type": "paragraph", "content": [{ "type": "text", "text": "cd" }] }),
                    ],
                    depth,
                );
                (
                    source,
                    vec![TypedOperation::JoinBlocks {
                        at: point_for_test(2),
                    }],
                )
            }
            3 => {
                let source = nested_doc(
                    vec![
                        json!({ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }),
                    ],
                    0,
                );
                (
                    source,
                    vec![TypedOperation::WrapInList {
                        range: range_for_test(0, 3),
                        list_type: "bulletList".into(),
                        item_type: "listItem".into(),
                        attrs: HashMap::new(),
                        item_attrs: HashMap::new(),
                    }],
                )
            }
            4 => {
                let source = nested_doc(
                    vec![json!({
                        "type": "bulletList",
                        "content": [
                            { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }] },
                            { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }] }
                        ]
                    })],
                    depth,
                );
                let at = rendered_scalar_offset(&source, &schema, "two") + 1;
                (
                    source,
                    vec![TypedOperation::IndentListItem {
                        at: point_for_test(at),
                    }],
                )
            }
            5 => {
                let source = nested_doc(
                    vec![json!({
                        "type": "bulletList",
                        "content": [{
                            "type": "listItem",
                            "content": [
                                { "type": "paragraph", "content": [{ "type": "text", "text": "outer" }] },
                                { "type": "bulletList", "content": [{ "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "inner" }] }] }] }
                            ]
                        }]
                    })],
                    depth,
                );
                let at = rendered_scalar_offset(&source, &schema, "inner") + 1;
                (
                    source,
                    vec![TypedOperation::OutdentListItem {
                        at: point_for_test(at),
                    }],
                )
            }
            6 => {
                let source = nested_doc(
                    vec![
                        json!({ "type": "bulletList", "content": [{ "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }] }] }),
                    ],
                    depth,
                );
                let at = rendered_scalar_offset(&source, &schema, "one") + 1;
                (
                    source,
                    vec![TypedOperation::UnwrapFromList {
                        at: point_for_test(at),
                    }],
                )
            }
            7 => {
                let source = nested_doc(
                    vec![json!({ "type": "paragraph", "content": [{ "type": "hardBreak" }] })],
                    depth,
                );
                (
                    source,
                    vec![TypedOperation::DeleteRange {
                        range: range_for_test(0, 1),
                    }],
                )
            }
            8 => {
                let source = nested_doc(
                    vec![json!({ "type": "paragraph", "content": [{ "type": "hardBreak" }] })],
                    depth,
                );
                (
                    source,
                    vec![TypedOperation::ReplaceRange {
                        range: range_for_test(0, 1),
                        content: Fragment::from(vec![Node::text(format!("seed-{seed}"), vec![])]),
                    }],
                )
            }
            9 => {
                let source = json!({
                    "type": "doc",
                    "content": [{
                        "type": "image",
                        "attrs": { "src": format!("old-{seed}"), "alt": "old alt" }
                    }]
                });
                (
                    source,
                    vec![TypedOperation::UpdateNodeAttrs {
                        at: point_for_test(0),
                        attrs: HashMap::from([
                            ("src".into(), Value::String(format!("new-{seed}"))),
                            ("alt".into(), Value::String("new alt".into())),
                            ("title".into(), Value::Null),
                            ("width".into(), Value::Null),
                            ("height".into(), Value::Null),
                        ]),
                    }],
                )
            }
            _ => {
                let source = nested_doc(
                    vec![
                        json!({ "type": "paragraph", "content": [{ "type": "text", "text": "A😀" }] }),
                        json!({ "type": "paragraph", "content": [{ "type": "text", "text": "B" }] }),
                    ],
                    depth,
                );
                let at = rendered_scalar_offset(&source, &schema, "B") - 1;
                (
                    source,
                    vec![TypedOperation::InsertNode {
                        at: point_for_test(at),
                        node: Node::void("horizontalRule".into(), HashMap::new()),
                    }],
                )
            }
        };
        let (actual, expected, _, update_len, estimate) = compile_and_execute(source, operations);
        assert_eq!(actual, expected, "fixed structural seed {seed}");
        assert!(update_len <= estimate, "fixed structural seed {seed}");
    }

    let source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "AB" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "sentinel" }] }
        ]
    });
    let (doc, schema, limits, mut compiled) = compile_operations_with_schema(
        &source,
        vec![TypedOperation::InsertNode {
            at: point_for_test(1),
            node: Node::void("hardBreak".into(), HashMap::new()),
        }],
        tiptap_schema(),
    );
    let sentinel_id = {
        let txn = doc.transact();
        txn.get_xml_fragment("prosemirror")
            .unwrap()
            .get(&txn, 1)
            .unwrap()
            .id()
    };
    {
        let txn = doc.transact();
        let preflight =
            preflight_mutation_work_for_test(71, &compiled.mutation_plan, &txn).unwrap();
        let exact = compiled.mutation_plan.compilation_work_for_test() + preflight;
        compiled.mutation_plan.set_work_limit_for_test(exact);
        preflight_mutation_plan(71, &compiled.mutation_plan, &txn).unwrap();
        compiled.mutation_plan.set_work_limit_for_test(exact - 1);
        assert_eq!(
            preflight_mutation_plan(71, &compiled.mutation_plan, &txn)
                .unwrap_err()
                .code,
            "OPERATION_LIMIT_EXCEEDED"
        );
        compiled.mutation_plan.set_work_limit_for_test(exact);
    }
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    assert_eq!(fragment.get(&txn, 1).unwrap().id(), sentinel_id);
    assert_eq!(
        YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap(),
        expected
    );
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

fn list_item_text<T: ReadTxn>(item: &XmlOut, txn: &T) -> XmlTextRef {
    let XmlOut::Element(item) = item else {
        panic!("list item expected")
    };
    let XmlOut::Element(paragraph) = item.get(txn, 0).unwrap() else {
        panic!("paragraph expected")
    };
    let XmlOut::Text(text) = paragraph.get(txn, 0).unwrap() else {
        panic!("text expected")
    };
    text
}
