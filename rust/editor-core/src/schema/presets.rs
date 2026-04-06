use std::collections::HashMap;

use crate::schema::content_rule::ContentRule;
use crate::schema::{AttrSpec, MarkSpec, NodeRole, NodeSpec, Schema};

/// Build the standard Tiptap schema using camelCase node names.
///
/// Node names: doc, paragraph, blockquote, bulletList, orderedList, listItem,
///             hardBreak, horizontalRule, image, text.
/// Mark names: bold, italic, underline, strike, link.
pub fn tiptap_schema() -> Schema {
    build_schema(NamingConvention::CamelCase)
}

/// Build the standard ProseMirror schema using snake_case node names.
///
/// Node names: doc, paragraph, blockquote, bullet_list, ordered_list, list_item,
///             hard_break, horizontal_rule, image, text.
/// Mark names: bold, italic, underline, strike, link.
pub fn prosemirror_schema() -> Schema {
    build_schema(NamingConvention::SnakeCase)
}

enum NamingConvention {
    CamelCase,
    SnakeCase,
}

/// Resolve a node name based on the naming convention.
///
/// The `camel` parameter is the camelCase name, `snake` is the snake_case
/// alternative. For names that are identical in both conventions (e.g.
/// "paragraph", "doc", "text"), pass the same value for both.
fn name(convention: &NamingConvention, camel: &str, snake: &str) -> String {
    match convention {
        NamingConvention::CamelCase => camel.to_string(),
        NamingConvention::SnakeCase => snake.to_string(),
    }
}

fn build_schema(convention: NamingConvention) -> Schema {
    let list_item_name = name(&convention, "listItem", "list_item");

    let nodes = vec![
        NodeSpec {
            name: "doc".to_string(),
            content: ContentRule::parse("block+").unwrap(),
            group: None,
            attrs: HashMap::new(),
            role: NodeRole::Doc,
            html_tag: None,
            is_void: false,
        },
        NodeSpec {
            name: "paragraph".to_string(),
            content: ContentRule::parse("inline*").unwrap(),
            group: Some("block".to_string()),
            attrs: HashMap::new(),
            role: NodeRole::TextBlock,
            html_tag: Some("p".to_string()),
            is_void: false,
        },
        NodeSpec {
            name: "blockquote".to_string(),
            content: ContentRule::parse("block+").unwrap(),
            group: Some("block".to_string()),
            attrs: HashMap::new(),
            role: NodeRole::Block,
            html_tag: Some("blockquote".to_string()),
            is_void: false,
        },
        NodeSpec {
            name: name(&convention, "bulletList", "bullet_list"),
            content: ContentRule::parse(&format!("{list_item_name}+")).unwrap(),
            group: Some("block".to_string()),
            attrs: HashMap::new(),
            role: NodeRole::List { ordered: false },
            html_tag: Some("ul".to_string()),
            is_void: false,
        },
        NodeSpec {
            name: name(&convention, "orderedList", "ordered_list"),
            content: ContentRule::parse(&format!("{list_item_name}+")).unwrap(),
            group: Some("block".to_string()),
            attrs: {
                let mut attrs = HashMap::new();
                attrs.insert(
                    "start".to_string(),
                    AttrSpec {
                        default: Some(serde_json::Value::Number(1.into())),
                    },
                );
                attrs
            },
            role: NodeRole::List { ordered: true },
            html_tag: Some("ol".to_string()),
            is_void: false,
        },
        NodeSpec {
            name: list_item_name,
            content: ContentRule::parse("paragraph block*").unwrap(),
            group: None,
            attrs: HashMap::new(),
            role: NodeRole::ListItem,
            html_tag: Some("li".to_string()),
            is_void: false,
        },
        NodeSpec {
            name: name(&convention, "hardBreak", "hard_break"),
            content: ContentRule::parse("").unwrap(),
            group: Some("inline".to_string()),
            attrs: HashMap::new(),
            role: NodeRole::HardBreak,
            html_tag: Some("br".to_string()),
            is_void: true,
        },
        NodeSpec {
            name: name(&convention, "horizontalRule", "horizontal_rule"),
            content: ContentRule::parse("").unwrap(),
            group: Some("block".to_string()),
            attrs: HashMap::new(),
            role: NodeRole::Block,
            html_tag: Some("hr".to_string()),
            is_void: true,
        },
        NodeSpec {
            name: "image".to_string(),
            content: ContentRule::parse("").unwrap(),
            group: Some("block".to_string()),
            attrs: {
                let mut attrs = HashMap::new();
                attrs.insert("src".to_string(), AttrSpec { default: None });
                attrs.insert("alt".to_string(), AttrSpec { default: None });
                attrs.insert("title".to_string(), AttrSpec { default: None });
                attrs.insert("width".to_string(), AttrSpec { default: None });
                attrs.insert("height".to_string(), AttrSpec { default: None });
                attrs
            },
            role: NodeRole::Block,
            html_tag: Some("img".to_string()),
            is_void: true,
        },
        NodeSpec {
            name: "text".to_string(),
            content: ContentRule::parse("").unwrap(),
            group: Some("inline".to_string()),
            attrs: HashMap::new(),
            role: NodeRole::Text,
            html_tag: None,
            is_void: false,
        },
    ];

    let marks = vec![
        MarkSpec {
            name: "bold".to_string(),
            attrs: HashMap::new(),
            excludes: None,
        },
        MarkSpec {
            name: "italic".to_string(),
            attrs: HashMap::new(),
            excludes: None,
        },
        MarkSpec {
            name: "underline".to_string(),
            attrs: HashMap::new(),
            excludes: None,
        },
        MarkSpec {
            name: "strike".to_string(),
            attrs: HashMap::new(),
            excludes: None,
        },
        MarkSpec {
            name: "link".to_string(),
            attrs: {
                let mut attrs = HashMap::new();
                attrs.insert("href".to_string(), AttrSpec { default: None });
                attrs
            },
            excludes: None,
        },
    ];

    Schema::new(nodes, marks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tiptap_has_all_expected_nodes() {
        let schema = tiptap_schema();
        let expected = [
            "doc",
            "paragraph",
            "blockquote",
            "bulletList",
            "orderedList",
            "listItem",
            "hardBreak",
            "horizontalRule",
            "image",
            "text",
        ];
        for name in &expected {
            assert!(
                schema.node(name).is_some(),
                "tiptap schema missing node '{name}'"
            );
        }
    }

    #[test]
    fn test_prosemirror_has_all_expected_nodes() {
        let schema = prosemirror_schema();
        let expected = [
            "doc",
            "paragraph",
            "blockquote",
            "bullet_list",
            "ordered_list",
            "list_item",
            "hard_break",
            "horizontal_rule",
            "image",
            "text",
        ];
        for name in &expected {
            assert!(
                schema.node(name).is_some(),
                "prosemirror schema missing node '{name}'"
            );
        }
    }

    #[test]
    fn test_both_schemas_have_all_marks() {
        for schema in &[tiptap_schema(), prosemirror_schema()] {
            for mark_name in &["bold", "italic", "underline", "strike", "link"] {
                assert!(
                    schema.mark(mark_name).is_some(),
                    "schema missing mark '{mark_name}'"
                );
            }
        }
    }

    #[test]
    fn test_ordered_list_has_start_attr() {
        let schema = tiptap_schema();
        let ol = schema.node("orderedList").unwrap();
        let start_attr = ol
            .attrs
            .get("start")
            .expect("orderedList should have 'start' attr");
        assert_eq!(
            start_attr.default,
            Some(serde_json::Value::Number(1.into()))
        );
    }

    #[test]
    fn test_block_group_membership() {
        let schema = tiptap_schema();
        let block_nodes = schema.nodes_in_group("block");
        let block_names: Vec<&str> = block_nodes.iter().map(|n| n.name.as_str()).collect();
        assert!(block_names.contains(&"paragraph"));
        assert!(block_names.contains(&"blockquote"));
        assert!(block_names.contains(&"bulletList"));
        assert!(block_names.contains(&"orderedList"));
        assert!(block_names.contains(&"horizontalRule"));
        assert!(!block_names.contains(&"doc"));
        assert!(!block_names.contains(&"text"));
    }

    #[test]
    fn test_inline_group_membership() {
        let schema = tiptap_schema();
        let inline_nodes = schema.nodes_in_group("inline");
        let inline_names: Vec<&str> = inline_nodes.iter().map(|n| n.name.as_str()).collect();
        assert!(inline_names.contains(&"text"));
        assert!(inline_names.contains(&"hardBreak"));
        assert!(!inline_names.contains(&"paragraph"));
    }
}
