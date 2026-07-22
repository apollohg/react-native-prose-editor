use crate::boundary::ResourceLimits;
use crate::schema::Schema;
use crate::tiptap_schema;
use crate::yrs_engine::{
    Affinity, EditingLimits, EditorOffsetKind, HistoryPolicy, InitializationMode,
    ResolvedSelection, RevisionedPosition, RevisionedRange, SelectionInput, SelectionIntent,
    TransactionOrigin, TypedCommand, TypedTransaction, YrsDocumentEngine, YrsEngineConfig,
};
use yrs::{diff_updates_v1, encode_state_vector_from_update_v1};

fn point(offset: u32) -> RevisionedPosition {
    RevisionedPosition {
        offset,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::Before,
    }
}

fn engine_with(
    schema: Schema,
    mode: InitializationMode,
    resource_limits: ResourceLimits,
    editing_limits: EditingLimits,
    max_length: Option<u32>,
) -> YrsDocumentEngine {
    YrsDocumentEngine::new(YrsEngineConfig {
        schema,
        fragment_name: "prosemirror".into(),
        initialization_mode: mode,
        resource_limits,
        editing_limits,
        max_length,
        scope: None,
    })
    .unwrap()
}

fn engine(mode: InitializationMode) -> YrsDocumentEngine {
    engine_with(
        tiptap_schema(),
        mode,
        ResourceLimits::default(),
        EditingLimits::default(),
        None,
    )
}

fn select_text(engine: &mut YrsDocumentEngine, request_id: u64, anchor: u32, head: u32) {
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent: SelectionIntent::Set(SelectionInput::Text {
                anchor: point(anchor),
                head: point(head),
            }),
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
}

fn select_node(engine: &mut YrsDocumentEngine, request_id: u64, at: u32) {
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent: SelectionIntent::Set(SelectionInput::Node { at: point(at) }),
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
}

#[derive(Debug, PartialEq)]
struct Audit {
    encoded: Vec<u8>,
    json: Option<serde_json::Value>,
    html: Option<String>,
    revision: u64,
    state_revision: u64,
    selection: Option<ResolvedSelection>,
    stored_marks: Option<Vec<crate::model::Mark>>,
    can_undo: bool,
    can_redo: bool,
    origin: Option<TransactionOrigin>,
}

fn audit(engine: &YrsDocumentEngine) -> Audit {
    Audit {
        encoded: engine.encoded_state().unwrap(),
        json: engine.document_json(),
        html: engine.document_html(),
        revision: engine.revision(),
        state_revision: engine.state_revision(),
        selection: engine.resolved_selection().cloned(),
        stored_marks: engine.stored_marks().map(<[_]>::to_vec),
        can_undo: engine.can_undo(),
        can_redo: engine.can_redo(),
        origin: engine.last_committed_origin(),
    }
}

fn dependent_text_updates() -> (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>) {
    let mut source = engine(InitializationMode::LocalEmpty);
    let base = source.encoded_state().unwrap();
    source
        .apply_command(1, TypedCommand::InsertText { text: "a".into() })
        .unwrap();
    let after_a = source.encoded_state().unwrap();
    source
        .apply_command(2, TypedCommand::InsertText { text: "b".into() })
        .unwrap();
    let after_b = source.encoded_state().unwrap();
    let base_sv = encode_state_vector_from_update_v1(&base).unwrap();
    let after_a_sv = encode_state_vector_from_update_v1(&after_a).unwrap();
    let delta_a = diff_updates_v1(&after_a, &base_sv).unwrap();
    let delta_b = diff_updates_v1(&after_b, &after_a_sv).unwrap();
    (base, delta_a, delta_b, after_b)
}

fn incompatible_blockquote_schema() -> Schema {
    Schema::from_json(&serde_json::json!({
        "nodes": [
            {"name":"doc","content":"block+","role":"doc"},
            {"name":"paragraph","content":"inline*","group":"block","role":"textBlock","htmlTag":"p"},
            {"name":"blockquote","content":"inline*","group":"block","role":"block","htmlTag":"blockquote"},
            {"name":"text","content":"","group":"inline","role":"text"}
        ],
        "marks": []
    }))
    .unwrap()
}

#[test]
fn out_of_order_updates_are_quarantined_until_dependencies_complete() {
    let (base, delta_a, delta_b, final_state) = dependent_text_updates();
    let mut target = engine(InitializationMode::AwaitRemote);
    let initial = audit(&target);

    let pending_b = target.apply_remote_update_v1(10, &delta_b).unwrap();
    assert!(!pending_b.changed);
    assert_eq!(audit(&target), initial);
    assert!(!target.is_ready());

    let pending_a = target.apply_remote_update_v1(11, &delta_a).unwrap();
    assert!(!pending_a.changed);
    assert_eq!(audit(&target), initial);

    let completed = target.apply_remote_update_v1(12, &base).unwrap();
    assert!(completed.changed);
    assert!(target.is_ready());
    assert_eq!(target.document().unwrap().root().text_content(), "ab");

    let mut expected = engine(InitializationMode::AwaitRemote);
    expected.apply_remote_update_v1(13, &final_state).unwrap();
    assert_eq!(
        target.encoded_state().unwrap(),
        expected.encoded_state().unwrap()
    );
    assert_eq!(target.document_json(), expected.document_json());
}

#[test]
fn delete_set_before_insert_is_quarantined_and_converges() {
    let mut source = engine(InitializationMode::LocalEmpty);
    source
        .apply_command(
            14,
            TypedCommand::InsertText {
                text: "delete-me".into(),
            },
        )
        .unwrap();
    let before_delete = source.encoded_state().unwrap();
    let before_delete_sv = encode_state_vector_from_update_v1(&before_delete).unwrap();
    source
        .apply_command(15, TypedCommand::DeleteBackward)
        .unwrap()
        .expect("delete must apply");
    let after_delete = source.encoded_state().unwrap();
    let delete_first = diff_updates_v1(&after_delete, &before_delete_sv).unwrap();

    let mut target = engine(InitializationMode::AwaitRemote);
    let initial = audit(&target);
    let pending = target.apply_remote_update_v1(16, &delete_first).unwrap();
    assert!(!pending.changed);
    assert_eq!(audit(&target), initial);
    assert!(!target.is_ready());

    assert!(
        target
            .apply_remote_update_v1(17, &before_delete)
            .unwrap()
            .changed
    );
    let mut expected = engine(InitializationMode::AwaitRemote);
    expected.apply_remote_update_v1(18, &after_delete).unwrap();
    assert_eq!(
        target.encoded_state().unwrap(),
        expected.encoded_state().unwrap()
    );
    assert_eq!(target.document_json(), expected.document_json());
    assert_eq!(target.document().unwrap().root().text_content(), "delete-m");
}

#[test]
fn deferred_limit_failure_discards_poison_and_allows_recovery() {
    let (base, delta_a, delta_b, _) = dependent_text_updates();
    let mut target = engine_with(
        tiptap_schema(),
        InitializationMode::AwaitRemote,
        ResourceLimits::default(),
        EditingLimits::default(),
        Some(1),
    );
    target.apply_remote_update_v1(20, &delta_b).unwrap();
    target.apply_remote_update_v1(21, &delta_a).unwrap();
    let before = audit(&target);
    let error = target.apply_remote_update_v1(22, &base).unwrap_err();
    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(error.details.as_ref().unwrap()["field"], "maxLength");
    assert_eq!(audit(&target), before);

    let mut valid = engine(InitializationMode::LocalEmpty);
    valid
        .apply_command(23, TypedCommand::InsertText { text: "z".into() })
        .unwrap();
    let recovered = target
        .apply_remote_update_v1(24, &valid.encoded_state().unwrap())
        .unwrap();
    assert!(recovered.changed);
    assert_eq!(target.document().unwrap().root().text_content(), "z");
}

#[test]
fn duplicate_corrupt_oversize_schema_and_node_limits_are_atomic() {
    let mut source = engine(InitializationMode::LocalEmpty);
    source
        .apply_command(
            30,
            TypedCommand::InsertText {
                text: "remote".into(),
            },
        )
        .unwrap();
    let update = source.encoded_state().unwrap();
    let mut target = engine(InitializationMode::AwaitRemote);
    target.apply_remote_update_v1(31, &update).unwrap();
    let admitted = audit(&target);
    let duplicate = target.apply_remote_update_v1(32, &update).unwrap();
    assert!(!duplicate.changed);
    assert_eq!(audit(&target), admitted);

    for corrupt in [&[0xff][..], &[1, 1][..], &[0, 1, 0xff][..]] {
        let before = audit(&target);
        let error = target.apply_remote_update_v1(33, corrupt).unwrap_err();
        assert_eq!(error.code, "DOCUMENT_INVALID");
        assert_eq!(error.details.as_ref().unwrap()["field"], "update");
        assert_eq!(audit(&target), before);
    }

    let tight_resources = ResourceLimits {
        max_encoded_state_bytes: 64,
        ..ResourceLimits::default()
    };
    let mut tight = engine_with(
        tiptap_schema(),
        InitializationMode::AwaitRemote,
        tight_resources,
        EditingLimits::default(),
        None,
    );
    let before = audit(&tight);
    let error = tight.apply_remote_update_v1(34, &[0; 65]).unwrap_err();
    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(
        error.details.as_ref().unwrap()["field"],
        "maxEncodedStateBytes"
    );
    assert_eq!(error.limit, Some(64));
    assert_eq!(error.actual, Some(65));
    assert_eq!(audit(&tight), before);

    let mut foreign = engine_with(
        incompatible_blockquote_schema(),
        InitializationMode::LocalEmpty,
        ResourceLimits::default(),
        EditingLimits::default(),
        None,
    );
    foreign
        .import_json(
            &serde_json::json!({"type":"doc","content":[{"type":"blockquote","content":[{"type":"text","text":"invalid in target"}]}]}).to_string(),
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let mut schema_target = engine(InitializationMode::AwaitRemote);
    let before = audit(&schema_target);
    let error = schema_target
        .apply_remote_update_v1(35, &foreign.encoded_state().unwrap())
        .unwrap_err();
    assert_eq!(error.code, "DOCUMENT_INVALID");
    assert_eq!(audit(&schema_target), before);

    let node_resources = ResourceLimits {
        max_document_nodes: 2,
        ..ResourceLimits::default()
    };
    let mut node_target = engine_with(
        tiptap_schema(),
        InitializationMode::AwaitRemote,
        node_resources,
        EditingLimits::default(),
        None,
    );
    let before = audit(&node_target);
    let error = node_target.apply_remote_update_v1(36, &update).unwrap_err();
    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(error.details.as_ref().unwrap()["field"], "update");
    assert_eq!(audit(&node_target), before);
}

#[test]
fn partial_position_maps_initialize_but_position_compilation_fails_closed() {
    for (request_id, content) in [
        (
            40,
            serde_json::json!([
                {
                    "type": "blockquote",
                    "content": [{ "type": "text", "text": "unmapped before" }]
                },
                {
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": "mapped" }]
                }
            ]),
        ),
        (
            41,
            serde_json::json!([
                {
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": "mapped" }]
                },
                {
                    "type": "blockquote",
                    "content": [{ "type": "text", "text": "unmapped after" }]
                }
            ]),
        ),
    ] {
        let mut source = engine_with(
            incompatible_blockquote_schema(),
            InitializationMode::LocalEmpty,
            ResourceLimits::default(),
            EditingLimits::default(),
            None,
        );
        let initial = serde_json::json!({ "type": "doc", "content": content });
        source
            .import_json(&initial.to_string(), TransactionOrigin::DocumentImport)
            .unwrap();
        let mut engine = engine_with(
            incompatible_blockquote_schema(),
            InitializationMode::AwaitRemote,
            ResourceLimits::default(),
            EditingLimits::default(),
            None,
        );
        let remote = engine
            .apply_remote_update_v1(request_id, &source.encoded_state().unwrap())
            .unwrap();
        assert!(remote.changed);

        let mut updated = initial;
        let paragraph = updated["content"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|node| node["type"] == "paragraph")
            .unwrap();
        paragraph["content"][0]["text"] = "mapped updated".into();
        source
            .import_json(&updated.to_string(), TransactionOrigin::DocumentImport)
            .unwrap();
        let follow_up = engine
            .apply_remote_update_v1(request_id + 10, &source.encoded_state().unwrap())
            .unwrap();
        assert!(follow_up.changed);
        assert!(engine
            .document()
            .unwrap()
            .root()
            .text_content()
            .contains("mapped updated"));
        let before = audit(&engine);

        let error = engine
            .apply_command(
                request_id + 100,
                TypedCommand::InsertText { text: "x".into() },
            )
            .unwrap_err();

        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
        assert_eq!(audit(&engine), before);
    }
}

#[test]
fn canonical_output_ceiling_accepts_exact_and_rejects_one_over_atomically() {
    let mut source = engine(InitializationMode::LocalEmpty);
    source
        .apply_command(
            40,
            TypedCommand::InsertText {
                text: "é😀".into()
            },
        )
        .unwrap();
    let update = source.encoded_state().unwrap();
    let exact_bytes = serde_json::to_vec(&source.document_json().unwrap())
        .unwrap()
        .len();

    let exact_limits = EditingLimits {
        max_derived_output_bytes: exact_bytes,
        ..EditingLimits::default()
    };
    let mut exact = engine_with(
        tiptap_schema(),
        InitializationMode::AwaitRemote,
        ResourceLimits::default(),
        exact_limits,
        None,
    );
    assert!(exact.apply_remote_update_v1(41, &update).unwrap().changed);

    let one_under_limits = EditingLimits {
        max_derived_output_bytes: exact_bytes - 1,
        ..EditingLimits::default()
    };
    let mut one_under = engine_with(
        tiptap_schema(),
        InitializationMode::AwaitRemote,
        ResourceLimits::default(),
        one_under_limits,
        None,
    );
    let before = audit(&one_under);
    let error = one_under.apply_remote_update_v1(42, &update).unwrap_err();
    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(
        error.details.as_ref().unwrap()["field"],
        "maxDerivedOutputBytes"
    );
    assert_eq!(error.limit, Some((exact_bytes - 1) as u64));
    assert_eq!(error.actual, Some(exact_bytes as u64));
    assert_eq!(audit(&one_under), before);
}

#[test]
fn remote_delete_of_selected_image_normalizes_selection_instead_of_rejecting() {
    let document = serde_json::json!({"type":"doc","content":[
        {"type":"image","attrs":{"src":"https://example.com/a.png","alt":null,"title":null,"width":10,"height":20}},
        {"type":"paragraph","content":[{"type":"text","text":"tail"}]}
    ]});
    let mut source = engine(InitializationMode::LocalEmpty);
    source
        .import_json(&document.to_string(), TransactionOrigin::DocumentImport)
        .unwrap();
    let mut target = engine(InitializationMode::AwaitRemote);
    target
        .apply_remote_update_v1(50, &source.encoded_state().unwrap())
        .unwrap();
    select_node(&mut target, 51, 0);
    assert!(matches!(
        target.resolved_selection(),
        Some(ResolvedSelection::Node { .. })
    ));

    source
        .apply_command(
            52,
            TypedCommand::DeleteRange {
                range: RevisionedRange {
                    from: point(0),
                    to: point(1),
                },
            },
        )
        .unwrap()
        .expect("image range deletion must apply");
    let commit = target
        .apply_remote_update_v1(53, &source.encoded_state().unwrap())
        .unwrap();
    assert!(commit.changed);
    assert!(matches!(
        target.resolved_selection(),
        Some(ResolvedSelection::Text { .. })
    ));
    assert_eq!(target.document().unwrap().root().text_content(), "tail");
}

#[test]
fn remote_insert_before_relative_cursor_preserves_local_stored_marks() {
    let document = serde_json::json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"base"}]}]});
    let mut source = engine(InitializationMode::LocalEmpty);
    source
        .import_json(&document.to_string(), TransactionOrigin::DocumentImport)
        .unwrap();
    let mut target = engine(InitializationMode::AwaitRemote);
    target
        .apply_remote_update_v1(60, &source.encoded_state().unwrap())
        .unwrap();

    select_text(&mut target, 61, 4, 4);
    target
        .apply_command(
            62,
            TypedCommand::ToggleMark {
                mark_type: "bold".into(),
            },
        )
        .unwrap()
        .unwrap();
    assert_eq!(target.stored_marks().unwrap()[0].mark_type(), "bold");

    select_text(&mut source, 63, 0, 0);
    source
        .apply_command(64, TypedCommand::InsertText { text: "R".into() })
        .unwrap();
    target
        .apply_remote_update_v1(65, &source.encoded_state().unwrap())
        .unwrap();
    assert_eq!(target.stored_marks().unwrap()[0].mark_type(), "bold");

    target
        .apply_command(66, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .unwrap();
    let json = target.document_json().unwrap().to_string();
    assert!(json.contains("\"text\":\"x\""));
    assert!(json.contains("\"type\":\"bold\""));
}

/// Task 6: prepare/commit split, sealing, and read-only state-vector/diff
/// encoding. Everything below is staging-only surface; the default-feature
/// test count of this file must stay unchanged.
mod staging {
    use super::*;
    use crate::yrs_engine::{DocumentScope, OperationError};
    use yrs::updates::decoder::Decode;
    use yrs::updates::encoder::Encode;
    use yrs::{Doc, GetString, ReadTxn, StateVector, Transact, Update};

    fn scoped_engine(mode: InitializationMode) -> YrsDocumentEngine {
        YrsDocumentEngine::new(YrsEngineConfig {
            schema: tiptap_schema(),
            fragment_name: "prosemirror".into(),
            initialization_mode: mode,
            resource_limits: ResourceLimits::default(),
            editing_limits: EditingLimits::default(),
            max_length: None,
            scope: Some(DocumentScope {
                document_id: "doc-remote".into(),
                lineage_id: "lineage-remote".into(),
            }),
        })
        .unwrap()
    }

    fn error_json(error: &OperationError) -> serde_json::Value {
        serde_json::to_value(error).unwrap()
    }

    /// Runs one update through both paths on twin engines and asserts full
    /// result and audit parity. Returns both engines for follow-up steps.
    fn assert_step_parity(
        one_shot: &mut YrsDocumentEngine,
        split: &mut YrsDocumentEngine,
        request_id: u64,
        update: &[u8],
    ) {
        let expected = one_shot.apply_remote_update_v1(request_id, update);
        let actual = split
            .prepare_remote_update_v1(request_id, update)
            .and_then(|prepared| split.commit_prepared_remote_update(prepared));
        match (&expected, &actual) {
            (Ok(expected_commit), Ok(actual_commit)) => {
                assert_eq!(expected_commit.changed, actual_commit.changed);
                assert_eq!(expected_commit.revision, actual_commit.revision);
            }
            (Err(expected_error), Err(actual_error)) => {
                assert_eq!(error_json(expected_error), error_json(actual_error));
            }
            (expected, actual) => {
                panic!("path divergence: one-shot {expected:?} vs prepare/commit {actual:?}");
            }
        }
        assert_eq!(audit(one_shot), audit(split));
        assert_eq!(one_shot.is_ready(), split.is_ready());
    }

    #[test]
    fn prepare_commit_matches_one_shot_across_the_full_admission_matrix() {
        // Valid + duplicate no-op.
        let mut source = engine(InitializationMode::LocalEmpty);
        source
            .apply_command(
                700,
                TypedCommand::InsertText {
                    text: "parity".into(),
                },
            )
            .unwrap();
        let valid = source.encoded_state().unwrap();
        let mut one_shot = engine(InitializationMode::AwaitRemote);
        let mut split = engine(InitializationMode::AwaitRemote);
        assert_step_parity(&mut one_shot, &mut split, 701, &valid);
        assert_step_parity(&mut one_shot, &mut split, 702, &valid);

        // Malformed bytes.
        for corrupt in [&[0xff][..], &[1, 1][..], &[0, 1, 0xff][..]] {
            assert_step_parity(&mut one_shot, &mut split, 703, corrupt);
        }

        // Over the encoded-state byte ceiling.
        let tight_resources = ResourceLimits {
            max_encoded_state_bytes: 64,
            ..ResourceLimits::default()
        };
        let mut tight_one_shot = engine_with(
            tiptap_schema(),
            InitializationMode::AwaitRemote,
            tight_resources.clone(),
            EditingLimits::default(),
            None,
        );
        let mut tight_split = engine_with(
            tiptap_schema(),
            InitializationMode::AwaitRemote,
            tight_resources,
            EditingLimits::default(),
            None,
        );
        assert_step_parity(&mut tight_one_shot, &mut tight_split, 704, &[0; 65]);

        // Dependency-pending quarantine, completion, and convergence.
        let (base, delta_a, delta_b, final_state) = dependent_text_updates();
        let mut one_shot = engine(InitializationMode::AwaitRemote);
        let mut split = engine(InitializationMode::AwaitRemote);
        assert_step_parity(&mut one_shot, &mut split, 705, &delta_b);
        assert!(!split.is_ready());
        assert_step_parity(&mut one_shot, &mut split, 706, &delta_a);
        assert_step_parity(&mut one_shot, &mut split, 707, &base);
        assert!(split.is_ready());
        assert_eq!(split.document().unwrap().root().text_content(), "ab");
        let mut expected = engine(InitializationMode::AwaitRemote);
        expected.apply_remote_update_v1(708, &final_state).unwrap();
        assert_eq!(
            split.encoded_state().unwrap(),
            expected.encoded_state().unwrap()
        );

        // Deferred over-ceiling failure discards quarantined poison on both
        // paths, then both recover identically.
        let (base, delta_a, delta_b, _) = dependent_text_updates();
        let deferred_engine = |mode| {
            engine_with(
                tiptap_schema(),
                mode,
                ResourceLimits::default(),
                EditingLimits::default(),
                Some(1),
            )
        };
        let mut one_shot = deferred_engine(InitializationMode::AwaitRemote);
        let mut split = deferred_engine(InitializationMode::AwaitRemote);
        assert_step_parity(&mut one_shot, &mut split, 709, &delta_b);
        assert_step_parity(&mut one_shot, &mut split, 710, &delta_a);
        assert_step_parity(&mut one_shot, &mut split, 711, &base);
        let mut valid = engine(InitializationMode::LocalEmpty);
        valid
            .apply_command(712, TypedCommand::InsertText { text: "z".into() })
            .unwrap();
        assert_step_parity(
            &mut one_shot,
            &mut split,
            713,
            &valid.encoded_state().unwrap(),
        );
        assert_eq!(split.document().unwrap().root().text_content(), "z");
    }

    #[test]
    fn prepared_update_rejects_after_a_local_edit_between_prepare_and_commit() {
        let mut source = engine(InitializationMode::LocalEmpty);
        source
            .apply_command(
                720,
                TypedCommand::InsertText {
                    text: "seed".into(),
                },
            )
            .unwrap();
        let seed = source.encoded_state().unwrap();
        let mut target = engine(InitializationMode::AwaitRemote);
        target.apply_remote_update_v1(721, &seed).unwrap();

        source
            .apply_command(722, TypedCommand::InsertText { text: "!".into() })
            .unwrap();
        let follow_up = source.encoded_state().unwrap();

        let prepared = target.prepare_remote_update_v1(723, &follow_up).unwrap();
        target
            .apply_command(
                724,
                TypedCommand::InsertText {
                    text: "local".into(),
                },
            )
            .unwrap()
            .expect("local edit applies");
        let before_commit = audit(&target);

        let error = target.commit_prepared_remote_update(prepared).unwrap_err();
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
        assert_eq!(error.request_id, 723);
        assert_eq!(audit(&target), before_commit);

        // A fresh prepare over the new state commits cleanly.
        let prepared = target.prepare_remote_update_v1(725, &follow_up).unwrap();
        assert!(
            target
                .commit_prepared_remote_update(prepared)
                .unwrap()
                .changed
        );
        assert!(target
            .document()
            .unwrap()
            .root()
            .text_content()
            .contains('!'));
    }

    #[test]
    fn prepared_update_rejects_after_a_second_remote_commit() {
        let mut source = engine(InitializationMode::LocalEmpty);
        source
            .apply_command(730, TypedCommand::InsertText { text: "a".into() })
            .unwrap();
        let first = source.encoded_state().unwrap();
        source
            .apply_command(731, TypedCommand::InsertText { text: "b".into() })
            .unwrap();
        let second = source.encoded_state().unwrap();

        let mut target = engine(InitializationMode::AwaitRemote);
        let prepared = target.prepare_remote_update_v1(732, &first).unwrap();
        target.apply_remote_update_v1(733, &second).unwrap();
        let before_commit = audit(&target);

        let error = target.commit_prepared_remote_update(prepared).unwrap_err();
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
        assert_eq!(error.request_id, 732);
        assert_eq!(audit(&target), before_commit);
        assert_eq!(target.document().unwrap().root().text_content(), "ab");
    }

    #[test]
    fn prepared_update_rejects_after_a_snapshot_restore() {
        let mut snapshot_source = scoped_engine(InitializationMode::LocalEmpty);
        snapshot_source
            .import_json(
                &serde_json::json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"snapshot"}]}]})
                    .to_string(),
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let snapshot = snapshot_source.export_snapshot().unwrap();

        let mut update_source = engine(InitializationMode::LocalEmpty);
        update_source
            .apply_command(
                740,
                TypedCommand::InsertText {
                    text: "remote".into(),
                },
            )
            .unwrap();
        let update = update_source.encoded_state().unwrap();

        let mut target = scoped_engine(InitializationMode::AwaitRemote);
        let prepared = target.prepare_remote_update_v1(741, &update).unwrap();
        assert!(target.restore_snapshot(&snapshot).unwrap().changed);
        let before_commit = audit(&target);

        let error = target.commit_prepared_remote_update(prepared).unwrap_err();
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
        assert_eq!(error.request_id, 741);
        assert_eq!(audit(&target), before_commit);
        assert_eq!(target.document().unwrap().root().text_content(), "snapshot");
    }

    #[test]
    fn prepared_remote_commits_stay_outside_local_undo_history() {
        // Without any local history, a prepared remote commit creates none.
        let mut source = engine(InitializationMode::LocalEmpty);
        source
            .apply_command(
                749,
                TypedCommand::InsertText {
                    text: "seed".into(),
                },
            )
            .unwrap();
        let mut fresh = engine(InitializationMode::AwaitRemote);
        let prepared = fresh
            .prepare_remote_update_v1(750, &source.encoded_state().unwrap())
            .unwrap();
        assert!(
            fresh
                .commit_prepared_remote_update(prepared)
                .unwrap()
                .changed
        );
        assert!(!fresh.can_undo());
        assert!(!fresh.can_redo());
        assert!(fresh.undo(751).unwrap().is_none());

        // With local history: twin engines run the same sequence through the
        // one-shot and the prepared path; the remote commit must not add an
        // undo group, must not mark local authorship, and undo/redo must stay
        // byte-identical across both paths.
        let build_target = || {
            let mut target = engine(InitializationMode::LocalEmpty);
            target
                .apply_command(
                    752,
                    TypedCommand::InsertText {
                        text: "local".into(),
                    },
                )
                .unwrap()
                .expect("local typing applies");
            assert!(target.can_undo());
            target
        };
        let mut one_shot = build_target();
        let mut split = build_target();

        // A genuine causal follow-up built on a peer that admitted our state.
        let remote_update_for = |target: &YrsDocumentEngine| {
            let mut peer = engine(InitializationMode::AwaitRemote);
            peer.apply_remote_update_v1(753, &target.encoded_state().unwrap())
                .unwrap();
            peer.apply_command(754, TypedCommand::InsertText { text: "R".into() })
                .unwrap()
                .expect("peer typing applies");
            peer.encoded_state().unwrap()
        };

        for (target, prepared_path) in [(&mut one_shot, false), (&mut split, true)] {
            let remote_update = remote_update_for(target);
            let local_clock_before = local_authored_clock(target);
            let commit = if prepared_path {
                let prepared = target
                    .prepare_remote_update_v1(755, &remote_update)
                    .unwrap();
                target.commit_prepared_remote_update(prepared).unwrap()
            } else {
                target.apply_remote_update_v1(755, &remote_update).unwrap()
            };
            assert!(commit.changed);
            assert_eq!(
                target.last_committed_origin(),
                Some(TransactionOrigin::RemoteSync)
            );
            assert!(target
                .document()
                .unwrap()
                .root()
                .text_content()
                .contains('R'));
            // No local-origin authorship: the local client's authored clock
            // is untouched by the remote commit.
            assert_eq!(local_authored_clock(target), local_clock_before);
            // The remote commit added no undo group: exactly the one local
            // group remains poppable.
            assert!(target.can_undo());
            target.undo(756).unwrap().expect("local undo applies");
            assert!(!target.can_undo(), "only the local group was poppable");
            assert!(target.can_redo());
            target.redo(757).unwrap().expect("redo applies");
            assert!(target
                .document()
                .unwrap()
                .root()
                .text_content()
                .contains('R'));
        }
        // Whatever the undo timeline semantics, the prepared path must not
        // drift from the one-shot path. The raw encoded states differ only by
        // the two engines' distinct client identities, so compare everything
        // derived instead.
        let mut one_shot_audit = audit(&one_shot);
        let mut split_audit = audit(&split);
        one_shot_audit.encoded.clear();
        split_audit.encoded.clear();
        assert_eq!(one_shot_audit, split_audit);
    }

    /// The local client's authored clock in the engine's state vector (0 when
    /// the client has authored nothing durable).
    fn local_authored_clock(engine: &YrsDocumentEngine) -> u32 {
        let encoded = engine.encoded_state().unwrap();
        if encoded.is_empty() {
            return 0;
        }
        let sv =
            StateVector::decode_v1(&encode_state_vector_from_update_v1(&encoded).unwrap()).unwrap();
        sv.iter()
            .find(|(client, _)| client.get() == engine.client_id())
            .map(|(_, clock)| *clock)
            .unwrap_or(0)
    }

    #[test]
    fn state_vector_and_diff_encoding_are_read_only_and_standard() {
        let mut engine = engine(InitializationMode::LocalEmpty);
        engine
            .import_json(
                &serde_json::json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"base"}]}]})
                    .to_string(),
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let base_state = engine.encoded_state().unwrap();
        engine
            .apply_command(
                760,
                TypedCommand::InsertText {
                    text: " grows".into(),
                },
            )
            .unwrap();
        let baseline = audit(&engine);

        // The encoded state vector equals the raw store's state vector.
        let encoded_sv = engine.encode_state_vector_v1(761).unwrap();
        assert_eq!(
            StateVector::decode_v1(&encoded_sv).unwrap(),
            StateVector::decode_v1(
                &encode_state_vector_from_update_v1(&engine.encoded_state().unwrap()).unwrap()
            )
            .unwrap()
        );

        // An independent raw yrs replica holding only the base state
        // reconstructs the exact document from the encoded diff.
        let replica = Doc::new();
        replica
            .transact_mut()
            .apply_update(Update::decode_v1(&base_state).unwrap())
            .unwrap();
        let replica_sv = replica.transact().state_vector().encode_v1();
        let diff = engine.encode_diff_v1(762, &replica_sv).unwrap();
        replica
            .transact_mut()
            .apply_update(Update::decode_v1(&diff).unwrap())
            .unwrap();
        assert_eq!(
            replica.transact().state_vector(),
            StateVector::decode_v1(&encoded_sv).unwrap()
        );
        let replica_text = {
            let txn = replica.transact();
            txn.get_xml_fragment("prosemirror")
                .unwrap()
                .get_string(&txn)
        };
        let engine_text = {
            let engine_state = engine.encoded_state().unwrap();
            let check = Doc::new();
            check
                .transact_mut()
                .apply_update(Update::decode_v1(&engine_state).unwrap())
                .unwrap();
            let txn = check.transact();
            txn.get_xml_fragment("prosemirror")
                .unwrap()
                .get_string(&txn)
        };
        assert_eq!(replica_text, engine_text);

        // An empty state vector yields the full state; an up-to-date state
        // vector yields a dependency-free no-op diff.
        let full = engine
            .encode_diff_v1(763, &StateVector::default().encode_v1())
            .unwrap();
        let fresh = Doc::new();
        fresh
            .transact_mut()
            .apply_update(Update::decode_v1(&full).unwrap())
            .unwrap();
        assert_eq!(
            fresh.transact().state_vector(),
            StateVector::decode_v1(&encoded_sv).unwrap()
        );
        let noop_diff = engine.encode_diff_v1(764, &encoded_sv).unwrap();
        assert!(Update::decode_v1(&noop_diff).is_ok());

        // Every encoding call above was read-only.
        assert_eq!(audit(&engine), baseline);
    }

    #[test]
    fn malformed_or_oversized_state_vector_input_rejects_with_structured_errors() {
        let mut source = engine(InitializationMode::LocalEmpty);
        source
            .apply_command(770, TypedCommand::InsertText { text: "sv".into() })
            .unwrap();
        let baseline = audit(&source);

        for corrupt in [&[0xff, 0xff, 0xff][..], &[0x01][..]] {
            let error = source.encode_diff_v1(771, corrupt).unwrap_err();
            assert_eq!(error.code, "DOCUMENT_INVALID");
            assert_eq!(error.request_id, 771);
            assert_eq!(error.details.as_ref().unwrap()["field"], "stateVector");
            assert_eq!(audit(&source), baseline);
        }

        let tight = engine_with(
            tiptap_schema(),
            InitializationMode::AwaitRemote,
            ResourceLimits {
                max_encoded_state_bytes: 64,
                ..ResourceLimits::default()
            },
            EditingLimits::default(),
            None,
        );
        let error = tight.encode_diff_v1(772, &[0; 65]).unwrap_err();
        assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
        assert_eq!(error.request_id, 772);
        assert_eq!(
            error.details.as_ref().unwrap()["field"],
            "maxEncodedStateBytes"
        );
        assert_eq!(error.limit, Some(64));
        assert_eq!(error.actual, Some(65));
    }

    /// Fix round 1: an *unchanged* snapshot restore still clears the
    /// dependency quarantine and rebinds the bounded history without touching
    /// revision/state/epoch or the store handle — it must nonetheless
    /// invalidate an outstanding prepared remote update (brief §2 lists
    /// snapshot restore as a rejecting intervening mutation, and an
    /// unsealed commit could panic mid-install on the rebound replay chain).
    #[test]
    fn prepared_update_rejects_after_an_unchanged_snapshot_restore() {
        let mut source = engine(InitializationMode::LocalEmpty);
        source
            .apply_command(
                780,
                TypedCommand::InsertText {
                    text: "seed".into(),
                },
            )
            .unwrap();
        let mut target = scoped_engine(InitializationMode::AwaitRemote);
        // A prior remote commit so the bounded replay chain already holds an
        // event (the reviewer's panic repro shape).
        target
            .apply_remote_update_v1(781, &source.encoded_state().unwrap())
            .unwrap();
        let snapshot = target.export_snapshot().unwrap();

        source
            .apply_command(782, TypedCommand::InsertText { text: "!".into() })
            .unwrap();
        let follow_up = source.encoded_state().unwrap();
        let prepared = target.prepare_remote_update_v1(783, &follow_up).unwrap();

        // Same-state restore takes the unchanged fast path.
        let restore = target.restore_snapshot(&snapshot).unwrap();
        assert!(!restore.changed);
        let before_commit = audit(&target);

        let error = target.commit_prepared_remote_update(prepared).unwrap_err();
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
        assert_eq!(error.request_id, 783);
        assert_eq!(audit(&target), before_commit);

        // A fresh prepare over the post-restore state commits cleanly.
        let prepared = target.prepare_remote_update_v1(784, &follow_up).unwrap();
        assert!(
            target
                .commit_prepared_remote_update(prepared)
                .unwrap()
                .changed
        );
        assert!(target
            .document()
            .unwrap()
            .root()
            .text_content()
            .contains('!'));
    }

    /// Fix round 1: the canonical-equal (no-op) import variant of the same
    /// hole — quarantine clear plus history rebind with no revision change.
    #[test]
    fn prepared_update_rejects_after_a_no_op_import() {
        let mut source = engine(InitializationMode::LocalEmpty);
        source
            .apply_command(
                790,
                TypedCommand::InsertText {
                    text: "seed".into(),
                },
            )
            .unwrap();
        let mut target = engine(InitializationMode::AwaitRemote);
        target
            .apply_remote_update_v1(791, &source.encoded_state().unwrap())
            .unwrap();
        let current_json = target.document_json().unwrap().to_string();

        source
            .apply_command(792, TypedCommand::InsertText { text: "!".into() })
            .unwrap();
        let follow_up = source.encoded_state().unwrap();
        let prepared = target.prepare_remote_update_v1(793, &follow_up).unwrap();

        // Importing the canonical-equal document takes the unchanged path.
        let import = target
            .import_json(&current_json, TransactionOrigin::DocumentImport)
            .unwrap();
        assert!(!import.changed);
        let before_commit = audit(&target);

        let error = target.commit_prepared_remote_update(prepared).unwrap_err();
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
        assert_eq!(error.request_id, 793);
        assert_eq!(audit(&target), before_commit);

        let prepared = target.prepare_remote_update_v1(794, &follow_up).unwrap();
        assert!(
            target
                .commit_prepared_remote_update(prepared)
                .unwrap()
                .changed
        );
        assert!(target
            .document()
            .unwrap()
            .root()
            .text_content()
            .contains('!'));
    }

    /// Task 7 no-echo extension: remote updates admitted through both the
    /// one-shot and the sealed prepare/commit paths never produce a
    /// collaboration outbox entry on an attached session, while an immediate
    /// local edit on the same session enqueues exactly one bounded entry.
    #[test]
    fn remote_updates_produce_no_outbox_entries_on_attached_sessions() {
        use crate::native_bridge_test_support::{self as bridge, SessionOptions};

        let mut source = scoped_engine(InitializationMode::LocalEmpty);
        source
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"remote"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();

        let id = bridge::create_session(SessionOptions {
            attach_runtime: true,
            ..SessionOptions::default()
        })
        .unwrap();

        // One-shot remote apply: no echo.
        let changed =
            bridge::apply_remote_update(id, 901, &source.encoded_state().unwrap()).unwrap();
        assert!(changed);
        assert_eq!(bridge::outbox_pending(id).unwrap(), Some((0, 0)));
        assert_eq!(bridge::last_reserved_upper_bound(id).unwrap(), None);

        // Sealed prepare/commit remote apply: no echo.
        source
            .apply_command(902, TypedCommand::InsertText { text: "!".into() })
            .unwrap();
        let follow_up = source.encoded_state().unwrap();
        let changed = bridge::apply_prepared_remote_update(id, 903, &follow_up).unwrap();
        assert!(changed);
        assert_eq!(bridge::outbox_pending(id).unwrap(), Some((0, 0)));

        // The same session still emits exactly one bounded entry for a local
        // trusted-origin edit.
        let base = bridge::session_audit(id).unwrap().document_revision;
        bridge::submit_input(
            id,
            &serde_json::json!({
                "version": 1,
                "requestId": 904,
                "baseDocumentRevision": base,
                "text": "local",
            })
            .to_string(),
        )
        .unwrap();
        let (count, bytes) = bridge::outbox_pending(id).unwrap().unwrap();
        assert_eq!(count, 1);
        assert!(bytes > 0);
        let bound = bridge::last_reserved_upper_bound(id).unwrap().unwrap();
        assert!(bytes <= bound);

        bridge::destroy_session(id);
    }
}
