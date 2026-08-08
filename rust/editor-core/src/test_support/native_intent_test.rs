use crate::boundary::ResourceLimits;
use crate::native_transaction_bridge::NativeTransactionBridge;
use crate::schema::presets::tiptap_schema;
use crate::session::{
    CollaborationLimits, DocumentState, EditorSession, EditorSessionConfig, SessionPolicy,
};
use crate::yrs_engine::{
    Affinity, EditingLimits, EditorOffsetKind, InitializationMode, ReplacementHistory,
    RevisionedPosition, SelectionInput, TransactionOrigin, TypedCommand, YrsDocumentEngine,
    YrsEngineConfig,
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

fn collaborative_session() -> EditorSession {
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
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abcd"}]}]}"#,
            ReplacementHistory::ResetAndClear,
        )
        .unwrap();
    session.attach_collaboration_runtime();
    session
}

fn scalar(offset: u32) -> RevisionedPosition {
    RevisionedPosition {
        offset,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::After,
    }
}

#[test]
fn command_uses_resolved_selection_without_an_intermediate_caret_commit() {
    let mut session = collaborative_session();
    let epoch = session
        .pin_position_epoch(11, session.engine.revision())
        .unwrap();
    let mut replica = engine(InitializationMode::AwaitRemote);
    replica
        .apply_remote_update_v1(2, &session.engine.encoded_state().unwrap())
        .unwrap();
    replica
        .apply_command(3, TypedCommand::InsertText { text: "R".into() })
        .unwrap();
    session
        .engine
        .apply_remote_update_v1(4, &replica.encoded_state().unwrap())
        .unwrap();
    replica
        .apply_command(5, TypedCommand::InsertText { text: "S".into() })
        .unwrap();
    session
        .engine
        .apply_remote_update_v1(6, &replica.encoded_state().unwrap())
        .unwrap();
    let resolved = session.resolve_epoch_range(11, epoch, 2, 2).unwrap();
    let revision_before = session.engine.revision();
    let state_revision_before = session.engine.state_revision();
    let outbox_before = session
        .collaboration_outbox()
        .unwrap()
        .pending_document_update_count();
    let selection = SelectionInput::Text {
        anchor: scalar(resolved.anchor),
        head: scalar(resolved.head),
    };

    let (engine, outbox) = session.engine_and_outbox();
    engine
        .apply_command_at_selection_with_outbox(
            7,
            TypedCommand::InsertText { text: "X".into() },
            selection,
            TransactionOrigin::LocalInput,
            outbox,
        )
        .unwrap();

    assert_eq!(session.engine.revision(), revision_before + 1);
    assert_eq!(session.engine.state_revision(), state_revision_before + 1);
    assert_eq!(
        session
            .collaboration_outbox()
            .unwrap()
            .pending_document_update_count(),
        outbox_before + 1,
    );
    assert_eq!(
        session.engine.document().unwrap().root().text_content(),
        "RSabXcd",
    );
}

#[test]
fn split_block_uses_the_pinned_multi_paragraph_scalar() {
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
        .pin_position_epoch(19, session.engine.revision())
        .unwrap();
    let request = serde_json::json!({
        "version": 1,
        "requestId": "2",
        "ownerId": "19",
        "positionEpoch": epoch.to_string(),
        "intent": {"type": "splitBlock", "anchor": 10, "head": 10},
    });

    NativeTransactionBridge::new(&mut session)
        .submit_native_intent(&request.to_string())
        .unwrap();

    assert_eq!(
        session.engine.document_html().unwrap(),
        "<p>Alpha</p><p>Beta</p><p></p><p>Gamma</p>",
    );
}

#[test]
fn set_selection_uses_the_pinned_multi_paragraph_scalar() {
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
        .pin_position_epoch(23, session.engine.revision())
        .unwrap();
    let request = serde_json::json!({
        "version": 1,
        "requestId": "2",
        "ownerId": "23",
        "positionEpoch": epoch.to_string(),
        "intent": {"type": "setSelection", "anchor": 10, "head": 10},
    });

    NativeTransactionBridge::new(&mut session)
        .submit_native_intent(&request.to_string())
        .unwrap();

    assert!(matches!(
        session.engine.resolved_selection(),
        Some(crate::yrs_engine::ResolvedSelection::Text { anchor, head })
            if anchor.scalar == 10 && head.scalar == 10
    ));
}
