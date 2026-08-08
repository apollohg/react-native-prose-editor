use crate::boundary::ResourceLimits;
use crate::schema::presets::tiptap_schema;
use crate::session::{
    CollaborationLimits, DocumentState, EditorSession, EditorSessionConfig, SessionPolicy,
};
use crate::yrs_engine::{
    Affinity, EditingLimits, EditorOffsetKind, HistoryPolicy, InitializationMode,
    ReplacementHistory, RevisionedPosition, SelectionInput, SelectionIntent, TransactionOrigin,
    TypedCommand, TypedTransaction, YrsDocumentEngine, YrsEngineConfig,
};

fn engine(mode: InitializationMode) -> YrsDocumentEngine {
    YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".into(),
        initialization_mode: mode,
        resource_limits: ResourceLimits::default(),
        editing_limits: EditingLimits::default(),
        max_length: None,
        scope: None,
    })
    .unwrap()
}

fn session_with_text(text: &str) -> EditorSession {
    let config = EditorSessionConfig::local_for_test();
    let mut session = EditorSession::new(
        engine(InitializationMode::LocalEmpty),
        SessionPolicy::from_config(&config),
        DocumentState::LocalReady,
        CollaborationLimits::default(),
    )
    .unwrap();
    let document = serde_json::json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{"type": "text", "text": text}],
        }],
    });
    session
        .replace_document_json(1, &document.to_string(), ReplacementHistory::ResetAndClear)
        .unwrap();
    session
}

fn point(offset: u32) -> RevisionedPosition {
    RevisionedPosition {
        offset,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::Before,
    }
}

fn insert_at(engine: &mut YrsDocumentEngine, request_id: u64, offset: u32, text: &str) {
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent: SelectionIntent::Set(SelectionInput::Text {
                anchor: point(offset),
                head: point(offset),
            }),
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    engine
        .apply_command(
            request_id + 1,
            TypedCommand::InsertText { text: text.into() },
        )
        .unwrap();
}

fn replica_of(session: &EditorSession) -> YrsDocumentEngine {
    let mut replica = engine(InitializationMode::AwaitRemote);
    replica
        .apply_remote_update_v1(10, &session.engine.encoded_state().unwrap())
        .unwrap();
    replica
}

fn apply_replica(session: &mut EditorSession, replica: &YrsDocumentEngine, request_id: u64) {
    session
        .engine
        .apply_remote_update_v1(request_id, &replica.encoded_state().unwrap())
        .unwrap();
}

fn selected_text(session: &EditorSession, anchor: u32, head: u32) -> String {
    let text = session.engine.document().unwrap().root().text_content();
    let start = anchor.min(head) as usize;
    let end = anchor.max(head) as usize;
    text.chars().skip(start).take(end - start).collect()
}

#[test]
fn epoch_range_resolves_after_multiple_remote_revisions() {
    let mut session = session_with_text("abcd");
    let epoch = session
        .pin_position_epoch(7, session.engine.revision())
        .unwrap();
    let mut replica = replica_of(&session);

    insert_at(&mut replica, 20, 2, "R");
    apply_replica(&mut session, &replica, 21);
    insert_at(&mut replica, 22, 0, "S");
    apply_replica(&mut session, &replica, 23);

    let resolved = session.resolve_epoch_range(7, epoch, 1, 3).unwrap();

    assert_eq!(
        selected_text(&session, resolved.anchor, resolved.head),
        "bRc"
    );
}

#[test]
fn epoch_preserves_reversed_unicode_selection() {
    let mut session = session_with_text("a😀bc");
    let epoch = session
        .pin_position_epoch(9, session.engine.revision())
        .unwrap();
    let mut replica = replica_of(&session);

    insert_at(&mut replica, 30, 0, "Ω");
    apply_replica(&mut session, &replica, 31);

    let resolved = session.resolve_epoch_range(9, epoch, 4, 1).unwrap();

    assert!(resolved.anchor > resolved.head);
    assert_eq!(
        selected_text(&session, resolved.anchor, resolved.head),
        "😀bc"
    );
}

#[test]
fn epoch_is_owner_scoped_and_release_is_terminal() {
    let mut session = session_with_text("abcd");
    let epoch = session
        .pin_position_epoch(11, session.engine.revision())
        .unwrap();

    let foreign = session.resolve_epoch_range(12, epoch, 1, 2).unwrap_err();
    assert_eq!(foreign.code, "POSITION_EPOCH_INVALID");

    session.release_position_epoch_owner(11);
    let released = session.resolve_epoch_range(11, epoch, 1, 2).unwrap_err();
    assert_eq!(released.code, "POSITION_EPOCH_INVALID");
}

#[test]
fn deleting_both_leaf_targets_resolves_through_structural_fallback() {
    let mut session = session_with_text("abcd");
    let epoch = session
        .pin_position_epoch(13, session.engine.revision())
        .unwrap();
    let mut replica = replica_of(&session);
    let replacement = serde_json::json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{"type": "text", "text": "z"}],
        }],
    });
    replica
        .prepare_root_replacement_json(
            40,
            &replacement.to_string(),
            ReplacementHistory::ResetAndClear,
        )
        .unwrap();
    apply_replica(&mut session, &replica, 41);

    let resolved = session.resolve_epoch_range(13, epoch, 1, 3).unwrap();

    assert!(resolved.fallback);
    assert!(resolved.anchor <= 1);
    assert!(resolved.head <= 1);
}

#[test]
fn replacing_an_owner_pin_invalidates_only_its_previous_epoch() {
    let mut session = session_with_text("abcd");
    let first = session
        .pin_position_epoch(15, session.engine.revision())
        .unwrap();
    let other = session
        .pin_position_epoch(16, session.engine.revision())
        .unwrap();
    let replacement = session
        .pin_position_epoch(15, session.engine.revision())
        .unwrap();

    assert_ne!(first, replacement);
    assert_eq!(
        session
            .resolve_epoch_range(15, first, 0, 0)
            .unwrap_err()
            .code,
        "POSITION_EPOCH_INVALID",
    );
    session.resolve_epoch_range(15, replacement, 0, 0).unwrap();
    session.resolve_epoch_range(16, other, 0, 0).unwrap();
}

#[test]
fn pinned_owner_count_is_bounded_without_evicting_live_epochs() {
    let mut session = session_with_text("a");
    let revision = session.engine.revision();
    let first = session.pin_position_epoch(1, revision).unwrap();
    for owner_id in 2..=64 {
        session.pin_position_epoch(owner_id, revision).unwrap();
    }

    let error = session.pin_position_epoch(65, revision).unwrap_err();

    assert_eq!(error.code, "POSITION_EPOCH_LIMIT_EXCEEDED");
    session.resolve_epoch_range(1, first, 0, 1).unwrap();
}

#[test]
fn unchanged_multi_paragraph_boundary_resolves_to_the_same_scalar() {
    let config = EditorSessionConfig::local_for_test();
    let mut session = EditorSession::new(
        engine(InitializationMode::LocalEmpty),
        SessionPolicy::from_config(&config),
        DocumentState::LocalReady,
        CollaborationLimits::default(),
    )
    .unwrap();
    session
        .replace_document_json(
            1,
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"Alpha"}]},{"type":"paragraph","content":[{"type":"text","text":"Beta"}]},{"type":"paragraph","content":[{"type":"text","text":"Gamma"}]}]}"#,
            ReplacementHistory::ResetAndClear,
        )
        .unwrap();
    let epoch = session
        .pin_position_epoch(17, session.engine.revision())
        .unwrap();

    let resolved = session.resolve_epoch_range(17, epoch, 10, 10).unwrap();

    assert_eq!(resolved.anchor, 10);
    assert_eq!(resolved.head, 10);
}
