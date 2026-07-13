use editor_core::boundary::ResourceLimits;
use editor_core::schema::Schema;
use editor_core::tiptap_schema;
use editor_core::yrs_engine::{
    DocumentScope, InitializationMode, YrsDocumentEngine, YrsEngineConfig,
};
use yrs::types::xml::XmlFragment;
use yrs::updates::decoder::Decode;
use yrs::{Doc, ReadTxn, Transact, Update};

fn local_config(schema: Schema) -> YrsEngineConfig {
    YrsEngineConfig {
        schema,
        fragment_name: "prosemirror".to_string(),
        initialization_mode: InitializationMode::LocalEmpty,
        resource_limits: ResourceLimits::default(),
        scope: None,
    }
}

fn custom_root_schema() -> Schema {
    Schema::from_json(&serde_json::json!({
        "nodes": [
            {
                "name": "article",
                "content": "body+",
                "role": "doc"
            },
            {
                "name": "body",
                "content": "inline*",
                "group": "body",
                "role": "textBlock",
                "htmlTag": "section"
            },
            {
                "name": "text",
                "group": "inline",
                "role": "text"
            }
        ],
        "marks": []
    }))
    .unwrap()
}

#[test]
fn local_empty_seeds_the_canonical_schema_document_at_revision_zero() {
    let engine = YrsDocumentEngine::new(local_config(tiptap_schema())).unwrap();
    let second_engine = YrsDocumentEngine::new(local_config(tiptap_schema())).unwrap();

    assert!(engine.is_ready());
    assert_eq!(engine.revision(), 0);
    assert_eq!(engine.last_committed_origin(), None);
    assert_eq!(
        engine.document_json(),
        Some(serde_json::json!({
            "type": "doc",
            "content": [{"type": "paragraph"}]
        }))
    );
    assert_eq!(engine.document_html().as_deref(), Some("<p></p>"));
    assert!(engine.document().is_some());
    assert!(!engine.encoded_state().unwrap().is_empty());
    assert_ne!(engine.client_id(), second_engine.client_id());
}

#[test]
fn await_remote_has_no_display_fallback_and_no_seeded_items() {
    let engine = YrsDocumentEngine::new(YrsEngineConfig {
        initialization_mode: InitializationMode::AwaitRemote,
        ..local_config(tiptap_schema())
    })
    .unwrap();

    assert!(!engine.is_ready());
    assert!(engine.document().is_none());
    assert!(engine.document_json().is_none());
    assert!(engine.document_html().is_none());
    assert_eq!(engine.revision(), 0);
    assert_eq!(engine.last_committed_origin(), None);
    assert!(engine.encoded_state().unwrap().is_empty());
}

#[test]
fn local_empty_respects_custom_roots_fragments_scope_and_limits() {
    let scope = DocumentScope {
        document_id: "document-7".to_string(),
        lineage_id: "lineage-3".to_string(),
    };
    let limits = ResourceLimits::default();
    let engine = YrsDocumentEngine::new(YrsEngineConfig {
        schema: custom_root_schema(),
        fragment_name: "article-content".to_string(),
        initialization_mode: InitializationMode::LocalEmpty,
        resource_limits: limits.clone(),
        scope: Some(scope.clone()),
    })
    .unwrap();

    let encoded_state = engine.encoded_state().unwrap();
    assert_eq!(
        engine.document_json(),
        Some(serde_json::json!({
            "type": "article",
            "content": [{"type": "body"}]
        }))
    );
    assert_eq!(
        engine.document_html().as_deref(),
        Some("<section></section>")
    );
    assert_eq!(engine.fragment_name(), "article-content");
    assert_eq!(engine.scope(), Some(&scope));
    assert_eq!(engine.resource_limits(), &limits);
    assert_eq!(engine.schema_fingerprint().len(), 64);

    let replay = Doc::new();
    replay
        .transact_mut()
        .apply_update(Update::decode_v1(&encoded_state).unwrap())
        .unwrap();
    let txn = replay.transact();
    let fragment = txn.get_xml_fragment("article-content").unwrap();
    assert_eq!(fragment.len(&txn), 1);
    assert!(txn.get_xml_fragment("prosemirror").is_none());
}

#[test]
fn local_empty_rejects_seeded_state_above_the_encoded_state_limit() {
    let engine = YrsDocumentEngine::new(local_config(tiptap_schema())).unwrap();
    let actual = engine.encoded_state().unwrap().len();
    let limit = actual - 1;
    let mut last_actual = actual;

    for _ in 0..128 {
        let mut config = local_config(tiptap_schema());
        config.resource_limits.max_encoded_state_bytes = limit;
        match YrsDocumentEngine::new(config) {
            Err(error) if error.actual == Some(actual) => {
                assert_eq!(error.code, "INPUT_LIMIT_EXCEEDED");
                assert_eq!(error.limit, Some(limit));
                assert_eq!(error.actual, Some(actual));
                return;
            }
            Err(error) => last_actual = error.actual.unwrap(),
            Ok(engine) => last_actual = engine.encoded_state().unwrap().len(),
        }
    }

    panic!(
        "could not sample a fresh client ID with encoded size {actual}; last size was {last_actual}"
    );
}
