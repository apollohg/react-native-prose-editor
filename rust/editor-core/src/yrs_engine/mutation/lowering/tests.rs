#[cfg(test)]
fn projected_textblock_test_schema() -> Schema {
    Schema::from_json(&serde_json::json!({
        "nodes": [
            { "name": "doc", "content": "block+", "role": "doc" },
            {
                "name": "info-box", "content": "inline*", "group": "block",
                "role": "textBlock",
                "json": { "type": "callout", "attrs": { "tone": "info" } }
            },
            { "name": "text", "content": "", "group": "inline", "role": "text" }
        ],
        "marks": []
    }))
    .unwrap()
}

include!("tests/prepared_batches.rs");

include!("tests/localized.rs");

include!("tests/import_diagnostics.rs");
