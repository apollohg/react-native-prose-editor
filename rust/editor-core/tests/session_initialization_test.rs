#![cfg(feature = "ffi-v2-staging")]

use std::sync::{Mutex, MutexGuard, OnceLock};

use editor_core::boundary::ResourceLimits;
use editor_core::session_initialization_test_support::{
    can_undo, create_local_empty, create_local_html, create_local_json, create_room_from_json,
    destroy_session, document_state, get_content_snapshot, get_html, get_json, has_document_state,
    registry_count, render_state, request_edit, transport_state, DocumentState, RenderState,
    TransportState,
};
use editor_core::tiptap_schema;
use editor_core::yrs_engine::{
    DocumentScope, EditingLimits, InitializationMode, TransactionOrigin, YrsDocumentEngine,
    YrsEngineConfig,
};

const JSON_DOCUMENT: &str = r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"created as JSON"}]}]}"#;

fn test_guard() -> MutexGuard<'static, ()> {
    static GUARD: OnceLock<Mutex<()>> = OnceLock::new();
    GUARD
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn snapshot_source() -> editor_core::yrs_engine::DocumentSnapshot {
    let mut source = YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".into(),
        initialization_mode: InitializationMode::LocalEmpty,
        resource_limits: ResourceLimits::default(),
        editing_limits: EditingLimits::default(),
        max_length: None,
        scope: Some(DocumentScope {
            document_id: "room-document".into(),
            lineage_id: "room-lineage".into(),
        }),
    })
    .unwrap();
    source
        .import_json(JSON_DOCUMENT, TransactionOrigin::DocumentImport)
        .unwrap();
    source.export_snapshot().unwrap()
}

#[test]
fn local_creation_admits_empty_json_and_html_before_registry_publication() {
    let _guard = test_guard();
    let baseline = registry_count();
    let invalid = create_local_json(r#"{"type":"doc","content":["#).unwrap_err();
    assert_eq!(invalid.domain, "document");
    assert_eq!(invalid.code, "DOCUMENT_INVALID");
    assert_eq!(registry_count(), baseline);

    let empty = create_local_empty().unwrap();
    assert_eq!(document_state(empty).unwrap(), DocumentState::LocalReady);
    assert_eq!(transport_state(empty).unwrap(), TransportState::Detached);
    assert_eq!(render_state(empty).unwrap(), RenderState::Ready);
    assert_eq!(get_html(empty).unwrap(), "<p></p>");
    assert_eq!(
        get_content_snapshot(empty).unwrap(),
        serde_json::json!({
            "html": get_html(empty).unwrap(),
            "json": get_json(empty).unwrap(),
        })
    );
    assert!(!can_undo(empty).unwrap());

    let json = create_local_json(JSON_DOCUMENT).unwrap();
    assert_eq!(
        get_json(json).unwrap(),
        serde_json::from_str::<serde_json::Value>(JSON_DOCUMENT).unwrap()
    );
    assert_eq!(get_html(json).unwrap(), "<p>created as JSON</p>");
    assert_eq!(
        get_content_snapshot(json).unwrap(),
        serde_json::json!({
            "html": get_html(json).unwrap(),
            "json": get_json(json).unwrap(),
        })
    );
    assert!(!can_undo(json).unwrap());

    let html = create_local_html("<h2>Created <strong>as HTML</strong></h2>").unwrap();
    assert_eq!(
        get_html(html).unwrap(),
        "<h2>Created <strong>as HTML</strong></h2>"
    );
    assert_eq!(
        get_content_snapshot(html).unwrap(),
        serde_json::json!({
            "html": get_html(html).unwrap(),
            "json": get_json(html).unwrap(),
        })
    );
    assert!(!can_undo(html).unwrap());

    assert!(request_edit(empty, 10, "empty edit").unwrap().can_undo);
    assert!(request_edit(json, 11, " json edit").unwrap().can_undo);
    assert!(request_edit(html, 12, " html edit").unwrap().can_undo);

    for id in [empty, json, html] {
        destroy_session(id);
    }
    assert_eq!(registry_count(), baseline);
}

#[test]
fn room_without_snapshot_is_loading_empty_and_rejects_local_work() {
    let _guard = test_guard();
    let id = create_room_from_json(r#"{"documentId":"room-document","lineageId":"room-lineage"}"#)
        .unwrap();

    assert_eq!(document_state(id).unwrap(), DocumentState::AwaitRemote);
    assert_eq!(transport_state(id).unwrap(), TransportState::Disconnected);
    assert_eq!(render_state(id).unwrap(), RenderState::Loading);
    assert!(!has_document_state(id).unwrap());

    for error in [get_json(id).unwrap_err(), get_html(id).unwrap_err()] {
        assert_eq!(error.domain, "operation");
        assert_eq!(error.code, "ENGINE_NOT_READY");
    }
    let edit_error = request_edit(id, 91, "must not be seeded").unwrap_err();
    assert_eq!(edit_error.domain, "operation");
    assert_eq!(edit_error.code, "ENGINE_NOT_READY");
    assert_eq!(edit_error.request_id, Some(91));
    let snapshot_error = get_content_snapshot(id).unwrap_err();
    assert_eq!(snapshot_error.domain, "operation");
    assert_eq!(snapshot_error.code, "ENGINE_NOT_READY");

    destroy_session(id);
}

#[test]
fn room_with_exact_snapshot_is_ready_and_disconnected_at_publication() {
    let _guard = test_guard();
    let snapshot = snapshot_source();
    let config = serde_json::json!({
        "documentId": snapshot.document_id,
        "lineageId": snapshot.lineage_id,
        "snapshot": snapshot,
    });
    let id = create_room_from_json(&config.to_string()).unwrap();

    assert_eq!(document_state(id).unwrap(), DocumentState::RoomReady);
    assert_eq!(transport_state(id).unwrap(), TransportState::Disconnected);
    assert_eq!(render_state(id).unwrap(), RenderState::Ready);
    assert_eq!(
        get_json(id).unwrap(),
        serde_json::from_str::<serde_json::Value>(JSON_DOCUMENT).unwrap()
    );
    assert_eq!(
        get_content_snapshot(id).unwrap(),
        serde_json::json!({
            "html": get_html(id).unwrap(),
            "json": get_json(id).unwrap(),
        })
    );

    destroy_session(id);
}

#[test]
fn room_configuration_accepts_only_scope_and_an_optional_snapshot() {
    let _guard = test_guard();
    let baseline = registry_count();
    let forbidden = [
        serde_json::json!({
            "documentId": "room-document",
            "lineageId": "room-lineage",
            "initialDocumentJson": {"type": "doc", "content": []},
        }),
        serde_json::json!({
            "documentId": "room-document",
            "lineageId": "room-lineage",
            "initialEncodedState": [0, 1],
        }),
        serde_json::json!({
            "documentId": "room-document",
            "lineageId": "room-lineage",
            "rawState": [0, 1],
        }),
        serde_json::json!({
            "documentId": "room-document",
            "lineageId": "room-lineage",
            "clientId": 7,
        }),
    ];

    for config in forbidden {
        let error = create_room_from_json(&config.to_string()).unwrap_err();
        assert_eq!(error.domain, "boundary");
        assert_eq!(error.code, "CONFIG_INVALID");
        assert_eq!(registry_count(), baseline);
    }
}
