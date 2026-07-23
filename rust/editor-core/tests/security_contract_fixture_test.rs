//! Shared hostile-fixture contract against the v2 UniFFI boundary.
//!
//! Task 16C: ported from the deleted legacy `editor_*` ABI to the production
//! v2 ABI (`editor_v2_create` / `editor_v2_get_document_json` /
//! `editor_v2_destroy`). The fixtures and their expected error codes are
//! unchanged; the v2 boundary surfaces the same stable codes through the
//! structured `FfiError` envelope (domain `document` for these cases).

use std::fs;

fn fixtures() -> serde_json::Value {
    let default_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/tests/security-contract-fixtures.json");
    let path = std::env::var_os("SECURITY_FIXTURE_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or(default_path);
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn create(config: serde_json::Value) -> editor_core::ffi_v2::types::FfiJsonResult {
    editor_core::ffi_v2::editor::editor_v2_create(config.to_string(), None)
}

fn expect_create_error(result: editor_core::ffi_v2::types::FfiJsonResult, expected_code: &str) {
    assert!(
        result.value.is_none(),
        "hostile input must not create a session"
    );
    let error = result
        .error
        .expect("hostile input must produce a structured error");
    assert_eq!(error.code, expected_code);
    assert_eq!(error.domain, "document");
}

#[test]
fn shared_hostile_fixtures_execute_against_the_rust_v2_boundary() {
    let fixtures = fixtures();

    for name in ["unknownScriptMark", "missingImageSource"] {
        // Mark/attribute validation fires on the document-mutation path, not
        // the canonical import path, matching the legacy `set_json` contract.
        let created = create(serde_json::json!({
            "initialization": { "type": "localEmpty" }
        }));
        let created_value: serde_json::Value =
            serde_json::from_str(&created.value.expect("empty create must succeed")).unwrap();
        let id = created_value["editorId"].as_str().unwrap().to_string();
        let result = editor_core::ffi_v2::editor::editor_v2_replace_document(
            id.clone(),
            serde_json::json!({
                "version": 1,
                // v2 request IDs are canonical decimal strings; a numeric ID
                // is CONFIG_INVALID before the hostile document is admitted.
                "requestId": "1",
                "setJson": fixtures[name]["document"],
                "history": "undoableBoundary",
            })
            .to_string(),
        );
        assert!(
            result.value.is_none(),
            "{name} must not replace the document"
        );
        let error = result
            .error
            .expect("{name} must produce a structured error");
        assert_eq!(
            error.code,
            fixtures[name]["expectedErrorCode"].as_str().unwrap(),
            "{name}"
        );
        assert_eq!(error.domain, "document", "{name}");
        // The v2 frozen taxonomy deliberately maps mark/attribute validation
        // to DOCUMENT_INVALID; the fixture's distinguishing detail lives in
        // the message (legacy UNKNOWN_MARK -> "unknown mark type: \"script\"",
        // REQUIRED_ATTRIBUTE_MISSING -> "requires attribute 'src'").
        let expected_detail = if name == "unknownScriptMark" {
            "unknown mark type: \"script\""
        } else {
            "requires attribute 'src'"
        };
        assert!(
            error.message.contains(expected_detail),
            "{name}: message {:?} must contain {expected_detail:?}",
            error.message
        );
        let destroyed = editor_core::ffi_v2::editor::editor_v2_destroy(id);
        assert!(destroyed.error.is_none(), "{:?}", destroyed.error);
    }

    let count = fixtures["oversizedSchema"]["nodeCount"].as_u64().unwrap();
    let nodes: Vec<_> = (0..count)
        .map(|index| serde_json::json!({"name": format!("n{index}"), "role": "block"}))
        .collect();
    let oversized = create(serde_json::json!({
        "schema": { "nodes": nodes, "marks": [] },
        "initialization": { "type": "localEmpty" },
    }));
    expect_create_error(
        oversized,
        fixtures["oversizedSchema"]["expectedErrorCode"]
            .as_str()
            .unwrap(),
    );

    let article = &fixtures["customArticleRoot"];
    let created = create(serde_json::json!({
        "schema": article["schema"],
        "initialization": {
            "type": "localJson",
            "json": article["document"],
        },
    }));
    assert!(created.error.is_none(), "{:?}", created.error);
    let created_value: serde_json::Value =
        serde_json::from_str(&created.value.expect("custom article create must succeed")).unwrap();
    let id = created_value["editorId"].as_str().unwrap().to_string();

    let document_result = editor_core::ffi_v2::editor::editor_v2_get_document_json(id.clone());
    let document: serde_json::Value =
        serde_json::from_str(&document_result.value.expect("document query must succeed")).unwrap();
    assert_eq!(document["type"], article["expectedRoot"]);

    let destroyed = editor_core::ffi_v2::editor::editor_v2_destroy(id);
    assert!(destroyed.error.is_none(), "{:?}", destroyed.error);
}
