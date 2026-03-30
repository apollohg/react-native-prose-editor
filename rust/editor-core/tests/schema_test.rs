use editor_core::schema::content_rule::ContentRule;
use editor_core::schema::presets::{prosemirror_schema, tiptap_schema};
use editor_core::schema::NodeRole;

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
