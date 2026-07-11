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
    assert!(rule.matches(&["block"], |child, symbol| *child == symbol));
    assert!(rule.matches(&["block", "block"], |child, symbol| *child == symbol));
    assert!(!rule.matches(&[] as &[&str], |child, symbol| *child == symbol));

    let rule = ContentRule::parse("inline*").unwrap();
    assert!(rule.matches(&[] as &[&str], |child, symbol| *child == symbol));
    assert!(rule.matches(&["inline", "inline"], |child, symbol| *child == symbol));

    let rule = ContentRule::parse("paragraph block*").unwrap();
    assert!(rule.matches(&["paragraph"], |child, symbol| *child == symbol));
    assert!(rule.matches(&["paragraph", "block"], |child, symbol| *child == symbol));
    assert!(!rule.matches(&["block"], |child, symbol| *child == symbol));
}

#[test]
fn content_rule_rejects_malformed_ranges_and_syntax() {
    for expression in [
        "block{2,1}",
        "block{,2}",
        "block{}",
        "block{2",
        "block|",
        "(block",
        "block)",
        "block**",
    ] {
        assert!(
            ContentRule::parse(expression).is_err(),
            "expected {expression:?} to be rejected"
        );
    }
}

#[test]
fn schema_rejects_duplicate_node_and_mark_names() {
    let duplicate_nodes = serde_json::json!({
        "nodes": [
            { "name": "doc", "content": "text*", "role": "doc" },
            { "name": "text", "role": "text" },
            { "name": "text", "role": "text" }
        ],
        "marks": []
    });
    assert!(Schema::from_json(&duplicate_nodes).is_err());

    let duplicate_marks = serde_json::json!({
        "nodes": [
            { "name": "doc", "content": "text*", "role": "doc" },
            { "name": "text", "role": "text" }
        ],
        "marks": [{ "name": "bold" }, { "name": "bold" }]
    });
    assert!(Schema::from_json(&duplicate_marks).is_err());
}

#[test]
fn schema_rejects_missing_or_duplicate_doc_and_text_roles() {
    for nodes in [
        serde_json::json!([{ "name": "text", "role": "text" }]),
        serde_json::json!([{ "name": "doc", "role": "doc" }]),
        serde_json::json!([
            { "name": "doc", "role": "doc" },
            { "name": "otherDoc", "role": "doc" },
            { "name": "text", "role": "text" }
        ]),
        serde_json::json!([
            { "name": "doc", "role": "doc" },
            { "name": "text", "role": "text" },
            { "name": "otherText", "role": "text" }
        ]),
    ] {
        assert!(Schema::from_json(&serde_json::json!({ "nodes": nodes, "marks": [] })).is_err());
    }
}

#[test]
fn schema_rejects_unresolved_content_symbols() {
    let result = Schema::from_json(&serde_json::json!({
        "nodes": [
            { "name": "doc", "content": "missing+", "role": "doc" },
            { "name": "text", "role": "text" }
        ],
        "marks": []
    }));
    assert!(result.is_err());
}

#[test]
fn schema_rejects_required_only_unconstructible_cycles() {
    let result = Schema::from_json(&serde_json::json!({
        "nodes": [
            { "name": "doc", "content": "a", "role": "doc" },
            { "name": "a", "content": "b", "group": "block" },
            { "name": "b", "content": "a", "group": "block" },
            { "name": "text", "role": "text" }
        ],
        "marks": []
    }));
    assert!(result.is_err());
}

#[test]
fn schema_accepts_cycles_when_the_cycle_is_optional() {
    let result = Schema::from_json(&serde_json::json!({
        "nodes": [
            { "name": "doc", "content": "recursive?", "role": "doc" },
            { "name": "recursive", "content": "recursive?", "group": "block" },
            { "name": "text", "role": "text" }
        ],
        "marks": []
    }));
    assert!(result.is_ok());
}

#[test]
fn schema_rejects_required_text_content_because_text_cannot_be_auto_created() {
    let result = Schema::from_json(&serde_json::json!({
        "nodes": [
            { "name": "doc", "content": "text", "role": "doc" },
            { "name": "text", "role": "text" }
        ],
        "marks": []
    }));
    assert!(result.is_err());
}

#[test]
fn schema_rejects_required_nodes_with_attributes_without_defaults() {
    let result = Schema::from_json(&serde_json::json!({
        "nodes": [
            { "name": "doc", "content": "media", "role": "doc" },
            { "name": "media", "attrs": { "src": {} }, "isVoid": true },
            { "name": "text", "role": "text" }
        ],
        "marks": []
    }));
    assert!(result.is_err());
}

#[test]
fn insertable_nodes_follow_the_actual_alternative_prefix() {
    let schema = Schema::from_json(&serde_json::json!({
        "nodes": [
            { "name": "doc", "content": "(title paragraph | image caption)", "role": "doc" },
            { "name": "title", "group": "block", "role": "textBlock" },
            { "name": "paragraph", "group": "block", "role": "block" },
            { "name": "image", "group": "block", "role": "block", "isVoid": true },
            { "name": "caption", "group": "block", "role": "block" },
            { "name": "text", "role": "text" }
        ],
        "marks": []
    }))
    .expect("schema");
    let doc = schema.node("doc").unwrap();

    let result = schema.insertable_nodes_at(doc, &["title"], &[]);
    assert_eq!(result, vec!["paragraph"]);
}

#[test]
fn insertable_nodes_validate_the_untouched_suffix() {
    let schema = Schema::from_json(&serde_json::json!({
        "nodes": [
            { "name": "doc", "content": "title (image title | paragraph)", "role": "doc" },
            { "name": "title", "group": "block", "role": "textBlock" },
            { "name": "paragraph", "group": "block", "role": "block" },
            { "name": "image", "group": "block", "role": "block", "isVoid": true },
            { "name": "text", "role": "text" }
        ],
        "marks": []
    }))
    .expect("schema");
    let result =
        schema.insertable_nodes_at(schema.node("doc").unwrap(), &["title"], &["paragraph"]);
    assert!(!result.contains(&"image".to_string()));
}

#[test]
fn preset_null_attribute_defaults_are_explicit_defaults() {
    let schema = tiptap_schema();
    let alt = &schema.node("image").unwrap().attrs["alt"];
    assert_eq!(alt.default, Some(serde_json::Value::Null));
}

#[test]
fn schema_rejects_default_construction_deeper_than_the_limit() {
    let mut nodes = vec![serde_json::json!({
        "name": "doc", "content": "n0", "role": "doc"
    })];
    for index in 0..129 {
        let content = if index == 128 {
            "".to_string()
        } else {
            format!("n{}", index + 1)
        };
        nodes.push(serde_json::json!({ "name": format!("n{index}"), "content": content }));
    }
    nodes.push(serde_json::json!({ "name": "text", "role": "text" }));
    assert!(Schema::from_json(&serde_json::json!({ "nodes": nodes, "marks": [] })).is_err());
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
            { "name": "paragraph", "content": "inline*", "group": "block", "role": "textBlock" },
            { "name": "text", "group": "inline", "role": "text" },
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
            { "name": "text", "group": "inline", "role": "text" },
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
fn list_item_type_for_considers_every_initial_alternative() {
    let schema = Schema::from_json(&serde_json::json!({
        "nodes": [
            { "name": "doc", "content": "block+", "role": "doc" },
            { "name": "paragraph", "content": "inline*", "group": "block", "role": "textBlock" },
            { "name": "text", "group": "inline", "role": "text" },
            { "name": "taskList", "content": "(decoration | taskGroup)+", "group": "block", "role": "list" },
            { "name": "decoration", "group": "block", "isVoid": true },
            { "name": "taskItem", "content": "paragraph+", "group": "taskGroup", "role": "listItem", "attrs": { "checked": { "default": false } } }
        ],
        "marks": []
    }))
    .expect("schema");

    assert_eq!(
        schema.list_item_type_for("taskList").as_deref(),
        Some("taskItem")
    );
}

/// Role helpers answer from NodeRole, not node names — a custom-named task
/// list/item must classify exactly like the presets do.
#[test]
fn schema_role_helpers_classify_by_role_not_name() {
    let schema = Schema::from_json(&serde_json::json!({
        "nodes": [
            { "name": "doc", "content": "block+", "role": "doc" },
            { "name": "paragraph", "content": "inline*", "group": "block", "role": "textBlock" },
            { "name": "text", "group": "inline", "role": "text" },
            { "name": "todoList", "content": "todoTask+", "group": "block", "role": "list" },
            { "name": "todoOrderedList", "content": "todoTask+", "group": "block", "role": "list" },
            {
                "name": "todoTask", "content": "paragraph+", "group": "block", "role": "listItem",
                "attrs": { "checked": { "default": false } }
            }
        ],
        "marks": []
    }))
    .expect("schema");

    assert!(schema.is_list("todoList"), "todoList should be a list");
    assert!(
        schema.is_list("todoOrderedList"),
        "todoOrderedList should be a list"
    );

    assert!(
        schema.is_ordered_list("todoOrderedList"),
        "todoOrderedList's name contains 'ordered', so from_json infers ordered=true"
    );
    assert!(
        !schema.is_ordered_list("todoList"),
        "todoList's name does not contain 'ordered', so it must not be ordered"
    );

    assert!(
        schema.is_list_item("todoTask"),
        "todoTask has role listItem"
    );
    assert!(
        !schema.is_list_item("todoList"),
        "todoList is a list container, not a list item"
    );

    for node_type in ["paragraph", "unknownNodeType"] {
        assert!(
            !schema.is_list(node_type),
            "{node_type} must not classify as a list"
        );
        assert!(
            !schema.is_ordered_list(node_type),
            "{node_type} must not classify as an ordered list"
        );
        assert!(
            !schema.is_list_item(node_type),
            "{node_type} must not classify as a list item"
        );
    }
}

#[test]
fn test_from_json_parses_mark_allow_undeclared_attrs_true() {
    let schema = Schema::from_json(&serde_json::json!({
        "nodes": [
            { "name": "doc", "content": "text*", "role": "doc" },
            { "name": "text", "group": "inline", "role": "text" }
        ],
        "marks": [
            { "name": "comment", "allowUndeclaredAttrs": true }
        ]
    }))
    .expect("schema JSON should parse");

    let comment = schema.mark("comment").expect("comment mark should exist");
    assert!(
        comment.allow_undeclared_attrs,
        "'allowUndeclaredAttrs: true' in mark spec JSON must set the flag"
    );
}

#[test]
fn test_from_json_defaults_mark_allow_undeclared_attrs_to_false_when_absent() {
    let schema = Schema::from_json(&serde_json::json!({
        "nodes": [
            { "name": "doc", "content": "text*", "role": "doc" },
            { "name": "text", "group": "inline", "role": "text" }
        ],
        "marks": [
            { "name": "bold" }
        ]
    }))
    .expect("schema JSON should parse");

    let bold = schema.mark("bold").expect("bold mark should exist");
    assert!(
        !bold.allow_undeclared_attrs,
        "absent 'allowUndeclaredAttrs' key on a mark spec must default to false"
    );
}

#[test]
fn test_from_json_defaults_allow_undeclared_attrs_to_false_when_absent() {
    let schema = Schema::from_json(&serde_json::json!({
        "nodes": [
            { "name": "doc", "content": "block+", "role": "doc" },
            { "name": "paragraph", "content": "inline*", "group": "block", "role": "textBlock", "htmlTag": "p" },
            { "name": "text", "group": "inline", "role": "text" }
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
