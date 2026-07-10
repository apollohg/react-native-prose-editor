use editor_core::schema::content_rule::ContentRule;
use editor_core::schema::presets::{prosemirror_schema, tiptap_schema};
use editor_core::schema::{NodeRole, Schema};

#[test]
fn test_schema_registers_node_types() {
    let schema = tiptap_schema();
    assert!(schema.node("paragraph").is_some());
    assert!(schema.node("bulletList").is_some());
    assert!(schema.node("nonexistent").is_none());
}

#[test]
fn test_schema_registers_mark_types() {
    let schema = tiptap_schema();
    assert!(schema.mark("bold").is_some());
    assert!(schema.mark("italic").is_some());
}

#[test]
fn test_node_role_assignment() {
    let schema = tiptap_schema();
    assert!(matches!(
        schema.node("paragraph").unwrap().role,
        NodeRole::TextBlock
    ));
    assert!(matches!(
        schema.node("bulletList").unwrap().role,
        NodeRole::List { ordered: false }
    ));
    assert!(matches!(
        schema.node("orderedList").unwrap().role,
        NodeRole::List { ordered: true }
    ));
}

#[test]
fn test_prosemirror_schema_uses_snake_case() {
    let schema = prosemirror_schema();
    assert!(schema.node("bullet_list").is_some());
    assert!(schema.node("ordered_list").is_some());
    assert!(schema.node("list_item").is_some());
    // camelCase should not exist
    assert!(schema.node("bulletList").is_none());
}

#[test]
fn test_content_rule_parsing() {
    let rule = ContentRule::parse("block+").unwrap();
    assert_eq!(rule.parts.len(), 1);
    assert_eq!(rule.parts[0].group, "block");
    assert_eq!(rule.parts[0].min, 1);
    assert_eq!(rule.parts[0].max, None);

    let rule = ContentRule::parse("inline*").unwrap();
    assert_eq!(rule.parts[0].min, 0);
    assert_eq!(rule.parts[0].max, None);

    let rule = ContentRule::parse("paragraph block*").unwrap();
    assert_eq!(rule.parts.len(), 2);
    assert_eq!(rule.parts[0].group, "paragraph");
    assert_eq!(rule.parts[0].min, 1);
    assert_eq!(rule.parts[1].group, "block");
    assert_eq!(rule.parts[1].min, 0);
}

#[test]
fn test_void_node_detection() {
    let schema = tiptap_schema();
    assert!(schema.node("horizontalRule").unwrap().is_void);
    assert!(schema.node("hardBreak").unwrap().is_void);
    assert!(!schema.node("paragraph").unwrap().is_void);
}

#[test]
fn test_preset_schemas_default_allow_undeclared_attrs_to_false() {
    for schema in [tiptap_schema(), prosemirror_schema()] {
        for node in schema.all_nodes() {
            assert!(
                !node.allow_undeclared_attrs,
                "preset node '{}' should default allow_undeclared_attrs to false",
                node.name
            );
        }
    }
}

#[test]
fn test_from_json_parses_allow_undeclared_attrs_true() {
    let schema = Schema::from_json(&serde_json::json!({
        "nodes": [
            { "name": "doc", "content": "block+", "role": "doc" },
            {
                "name": "mention",
                "content": "",
                "group": "inline",
                "role": "inline",
                "isVoid": true,
                "allowUndeclaredAttrs": true,
                "attrs": { "label": { "default": null } }
            }
        ],
        "marks": []
    }))
    .expect("schema JSON should parse");

    let mention = schema.node("mention").expect("mention node should exist");
    assert!(
        mention.allow_undeclared_attrs,
        "'allowUndeclaredAttrs: true' in schema JSON must set the flag"
    );
}

/// When a task list's content group contains BOTH a generic listItem and a
/// taskItem, the task list must resolve to the task item type — not the
/// alphabetically-first candidate.
#[test]
fn list_item_type_for_prefers_task_item_for_task_lists() {
    let schema = Schema::from_json(&serde_json::json!({
        "nodes": [
            { "name": "doc", "content": "block+", "role": "doc" },
            { "name": "paragraph", "content": "inline*", "group": "block", "role": "textBlock" },
            { "name": "text", "role": "text" },
            { "name": "bulletList", "content": "itemGroup+", "group": "block", "role": "list" },
            { "name": "taskList", "content": "itemGroup+", "group": "block", "role": "list" },
            { "name": "listItem", "content": "paragraph+", "group": "itemGroup", "role": "listItem" },
            {
                "name": "taskItem", "content": "paragraph+", "group": "itemGroup", "role": "listItem",
                "attrs": { "checked": { "default": false } }
            }
        ],
        "marks": []
    }))
    .expect("schema");

    assert_eq!(
        schema.list_item_type_for("taskList").as_deref(),
        Some("taskItem")
    );
    assert_eq!(
        schema.list_item_type_for("bulletList").as_deref(),
        Some("listItem")
    );
}

#[test]
fn test_from_json_defaults_allow_undeclared_attrs_to_false_when_absent() {
    let schema = Schema::from_json(&serde_json::json!({
        "nodes": [
            { "name": "doc", "content": "block+", "role": "doc" },
            { "name": "paragraph", "content": "inline*", "group": "block", "role": "textBlock", "htmlTag": "p" }
        ],
        "marks": []
    }))
    .expect("schema JSON should parse");

    let paragraph = schema
        .node("paragraph")
        .expect("paragraph node should exist");
    assert!(
        !paragraph.allow_undeclared_attrs,
        "absent 'allowUndeclaredAttrs' key must default to false"
    );
}
