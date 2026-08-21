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

fn collaborative_session_with_filter(input_filter: Option<&str>) -> EditorSession {
    let mut config = EditorSessionConfig::local_for_test();
    config.input_filter = input_filter.map(str::to_string);
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

fn collaborative_session() -> EditorSession {
    collaborative_session_with_filter(None)
}

fn scalar(offset: u32) -> RevisionedPosition {
    RevisionedPosition {
        offset,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::After,
    }
}

fn native_intent_request(
    session: &mut EditorSession,
    owner_id: u64,
    request_id: u64,
    intent: serde_json::Value,
) -> String {
    let epoch = session
        .pin_position_epoch(owner_id, session.engine.revision())
        .unwrap();
    serde_json::json!({
        "version": 1,
        "requestId": request_id.to_string(),
        "ownerId": owner_id.to_string(),
        "positionEpoch": epoch.to_string(),
        "intent": intent,
    })
    .to_string()
}

#[derive(Debug, PartialEq)]
struct SessionAudit {
    document_json: serde_json::Value,
    encoded_state: Vec<u8>,
    state_vector: Vec<u8>,
    document_revision: u64,
    state_revision: u64,
    can_undo: bool,
    can_redo: bool,
    selection: Option<String>,
    stored_marks: Option<String>,
    last_committed_origin: Option<String>,
    outbox_pending_updates: usize,
    outbox_pending_bytes: usize,
    outbox_reserved_messages: usize,
    outbox_reserved_bytes: usize,
    last_reserved_upper_bound: Option<usize>,
}

fn session_audit(session: &EditorSession) -> SessionAudit {
    let outbox = session.collaboration_outbox().unwrap();
    SessionAudit {
        document_json: session.engine.document_json().unwrap(),
        encoded_state: session.engine.encoded_state().unwrap(),
        state_vector: session.engine.encode_state_vector_v1(0).unwrap(),
        document_revision: session.engine.revision(),
        state_revision: session.engine.state_revision(),
        can_undo: session.engine.can_undo(),
        can_redo: session.engine.can_redo(),
        selection: session
            .engine
            .resolved_selection()
            .map(|selection| format!("{selection:?}")),
        stored_marks: session
            .engine
            .stored_marks()
            .map(|marks| format!("{marks:?}")),
        last_committed_origin: session
            .engine
            .last_committed_origin()
            .map(|origin| origin.as_tag().to_string()),
        outbox_pending_updates: outbox.pending_document_update_count(),
        outbox_pending_bytes: outbox.pending_document_update_bytes(),
        outbox_reserved_messages: outbox.reserved_messages(),
        outbox_reserved_bytes: outbox.reserved_bytes(),
        last_reserved_upper_bound: outbox.last_reserved_upper_bound_for_test(),
    }
}

#[test]
fn native_insert_text_applies_input_filter_per_character() {
    let mut session = collaborative_session_with_filter(Some("[0-9]"));
    let request = native_intent_request(
        &mut session,
        31,
        32,
        serde_json::json!({
            "type": "insertText",
            "anchor": 4,
            "head": 4,
            "text": "a1b2",
        }),
    );

    let outcome = NativeTransactionBridge::new(&mut session)
        .submit_native_intent(&request)
        .unwrap();
    let outcome: serde_json::Value = serde_json::from_str(&outcome).unwrap();

    assert_eq!(outcome["type"], "transaction");
    assert_eq!(outcome["changed"], true);
    assert_eq!(outcome["documentChanged"], true);
    assert_eq!(
        session.engine.document().unwrap().root().text_content(),
        "abcd12"
    );
    assert_eq!(
        session
            .collaboration_outbox()
            .unwrap()
            .pending_document_update_count(),
        1,
    );
}

#[test]
fn native_replace_selection_text_applies_input_filter_per_character() {
    let mut session = collaborative_session_with_filter(Some("[0-9]"));
    let request = native_intent_request(
        &mut session,
        33,
        34,
        serde_json::json!({
            "type": "replaceSelectionText",
            "anchor": 1,
            "head": 3,
            "text": "a1b2",
        }),
    );

    let outcome = NativeTransactionBridge::new(&mut session)
        .submit_native_intent(&request)
        .unwrap();
    let outcome: serde_json::Value = serde_json::from_str(&outcome).unwrap();

    assert_eq!(outcome["type"], "transaction");
    assert_eq!(outcome["changed"], true);
    assert_eq!(outcome["documentChanged"], true);
    assert_eq!(
        session.engine.document().unwrap().root().text_content(),
        "a12d"
    );
    assert_eq!(
        session
            .collaboration_outbox()
            .unwrap()
            .pending_document_update_count(),
        1,
    );
}

#[test]
fn fully_filtered_native_text_intents_are_atomic_unchanged_transactions() {
    for intent_type in ["insertText", "replaceSelectionText"] {
        let mut session = collaborative_session_with_filter(Some("[0-9]"));
        let request = native_intent_request(
            &mut session,
            35,
            36,
            serde_json::json!({
                "type": intent_type,
                "anchor": 1,
                "head": 3,
                "text": "abc",
            }),
        );
        let before = session_audit(&session);

        let outcome = NativeTransactionBridge::new(&mut session)
            .submit_native_intent(&request)
            .unwrap();
        let outcome: serde_json::Value = serde_json::from_str(&outcome).unwrap();

        assert_eq!(outcome["type"], "transaction", "{intent_type}");
        assert_eq!(outcome["changed"], false, "{intent_type}");
        assert_eq!(outcome["documentChanged"], false, "{intent_type}");
        assert_eq!(session_audit(&session), before, "{intent_type}");
    }
}

#[test]
fn same_text_native_replacement_reports_selection_change_without_document_change() {
    let mut session = collaborative_session();
    let request = native_intent_request(
        &mut session,
        41,
        42,
        serde_json::json!({
            "type": "replaceSelectionText",
            "anchor": 0,
            "head": 4,
            "text": "abcd",
        }),
    );
    let revision_before = session.engine.revision();
    let outbox_before = session
        .collaboration_outbox()
        .unwrap()
        .pending_document_update_count();

    let outcome = NativeTransactionBridge::new(&mut session)
        .submit_native_intent(&request)
        .unwrap();
    let outcome: serde_json::Value = serde_json::from_str(&outcome).unwrap();

    assert_eq!(outcome["type"], "transaction");
    assert_eq!(outcome["changed"], true);
    assert_eq!(outcome["documentChanged"], false);
    assert_eq!(session.engine.revision(), revision_before);
    assert_eq!(
        session
            .collaboration_outbox()
            .unwrap()
            .pending_document_update_count(),
        outbox_before,
    );
}

#[test]
fn invalid_native_text_intent_filter_is_atomic() {
    for intent_type in ["insertText", "replaceSelectionText"] {
        let mut session = collaborative_session_with_filter(Some("[unclosed"));
        let owner_id = 37;
        let request_id = 38;
        let request = native_intent_request(
            &mut session,
            owner_id,
            request_id,
            serde_json::json!({
                "type": intent_type,
                "anchor": 1,
                "head": 3,
                "text": "a1b2",
            }),
        );
        let before = session_audit(&session);

        let error = NativeTransactionBridge::new(&mut session)
            .submit_native_intent(&request)
            .unwrap_err();

        assert_eq!(error.code, "CONFIG_INVALID", "{intent_type}");
        assert_eq!(error.request_id, Some(request_id), "{intent_type}");
        assert_eq!(session_audit(&session), before, "{intent_type}");
        assert!(
            session
                .native_request_outcome(owner_id, request_id)
                .unwrap()
                .is_none(),
            "{intent_type}",
        );
    }
}

#[test]
fn native_input_filter_does_not_affect_non_text_intents() {
    let mut session = collaborative_session_with_filter(Some("[unclosed"));
    let request = native_intent_request(
        &mut session,
        39,
        40,
        serde_json::json!({
            "type": "deleteRange",
            "anchor": 1,
            "head": 2,
        }),
    );

    let outcome = NativeTransactionBridge::new(&mut session)
        .submit_native_intent(&request)
        .unwrap();
    let outcome: serde_json::Value = serde_json::from_str(&outcome).unwrap();

    assert_eq!(outcome["type"], "transaction");
    assert_eq!(outcome["changed"], true);
    assert_eq!(
        session.engine.document().unwrap().root().text_content(),
        "acd"
    );
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
