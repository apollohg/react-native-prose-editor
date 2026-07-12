use std::fs;

fn fixtures() -> serde_json::Value {
    let default_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/tests/security-contract-fixtures.json");
    let path = std::env::var_os("SECURITY_FIXTURE_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or(default_path);
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn shared_hostile_fixtures_execute_against_the_rust_boundary() {
    let fixtures = fixtures();
    let created: serde_json::Value =
        serde_json::from_str(&editor_core::editor_create_result("{}".into())).unwrap();
    let id = created["editorId"].as_u64().unwrap();

    for name in ["unknownScriptMark", "missingImageSource"] {
        let result: serde_json::Value = serde_json::from_str(&editor_core::editor_set_json(
            id,
            fixtures[name]["document"].to_string(),
        ))
        .unwrap();
        assert_eq!(result["error"]["code"], fixtures[name]["expectedErrorCode"]);
    }
    editor_core::editor_destroy(id);

    let count = fixtures["oversizedSchema"]["nodeCount"].as_u64().unwrap();
    let nodes: Vec<_> = (0..count)
        .map(|index| serde_json::json!({"name": format!("n{index}"), "role": "block"}))
        .collect();
    let oversized: serde_json::Value = serde_json::from_str(&editor_core::editor_create_result(
        serde_json::json!({"schema":{"nodes":nodes,"marks":[]}}).to_string(),
    ))
    .unwrap();
    assert_eq!(
        oversized["error"]["code"],
        fixtures["oversizedSchema"]["expectedErrorCode"]
    );

    let article = &fixtures["customArticleRoot"];
    let created: serde_json::Value = serde_json::from_str(&editor_core::editor_create_result(
        serde_json::json!({"schema": article["schema"]}).to_string(),
    ))
    .unwrap();
    let id = created["editorId"].as_u64().unwrap();
    let result: serde_json::Value = serde_json::from_str(&editor_core::editor_set_json(
        id,
        article["document"].to_string(),
    ))
    .unwrap();
    assert!(result.get("error").is_none(), "{result}");
    let document: serde_json::Value =
        serde_json::from_str(&editor_core::editor_get_json(id)).unwrap();
    assert_eq!(document["type"], article["expectedRoot"]);
    editor_core::editor_destroy(id);
}
