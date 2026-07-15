use editor_core::boundary::ResourceLimits;
use editor_core::schema::Schema;
use editor_core::tiptap_schema;
use editor_core::yrs_engine::{
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
    stored_marks: Option<Vec<editor_core::model::Mark>>,
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
