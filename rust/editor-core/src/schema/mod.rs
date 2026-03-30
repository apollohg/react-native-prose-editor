pub mod content_rule;
pub mod presets;

use std::collections::HashMap;

use crate::schema::content_rule::ContentRule;

/// A schema defines the set of node types and mark types available in a document.
///
/// Node and mark names are plain strings, allowing the same schema structure to
/// support different naming conventions (e.g. camelCase for Tiptap, snake_case
/// for ProseMirror).
#[derive(Debug, Clone)]
pub struct Schema {
    nodes: HashMap<String, NodeSpec>,
    marks: HashMap<String, MarkSpec>,
}

/// Specification for a node type within a schema.
#[derive(Debug, Clone)]
pub struct NodeSpec {
    pub name: String,
    pub content: ContentRule,
    pub group: Option<String>,
    pub attrs: HashMap<String, AttrSpec>,
    pub role: NodeRole,
    pub html_tag: Option<String>,
    /// If `true`, this node has no editable content (e.g. horizontal rule, hard break).
    pub is_void: bool,
}

/// The semantic role of a node, used by transactions and rendering to handle
/// node types generically without hardcoding names.
#[derive(Debug, Clone)]
pub enum NodeRole {
    Doc,
    TextBlock,
    List { ordered: bool },
    ListItem,
    Text,
    HardBreak,
    Inline,
    Block,
}

/// Specification for a mark type within a schema.
#[derive(Debug, Clone)]
pub struct MarkSpec {
    pub name: String,
    pub attrs: HashMap<String, AttrSpec>,
    /// Marks in the `excludes` set cannot coexist with this mark on the same
    /// text range. `None` means no exclusions.
    pub excludes: Option<String>,
}

/// Specification for a single attribute on a node or mark type.
#[derive(Debug, Clone)]
pub struct AttrSpec {
    pub default: Option<serde_json::Value>,
}

impl Schema {
    /// Create a schema from lists of node and mark specs.
    pub fn new(nodes: Vec<NodeSpec>, marks: Vec<MarkSpec>) -> Self {
        Self {
            nodes: nodes.into_iter().map(|n| (n.name.clone(), n)).collect(),
            marks: marks.into_iter().map(|m| (m.name.clone(), m)).collect(),
        }
    }

    /// Look up a node spec by name.
    pub fn node(&self, name: &str) -> Option<&NodeSpec> {
        self.nodes.get(name)
    }

    /// Look up a mark spec by name.
    pub fn mark(&self, name: &str) -> Option<&MarkSpec> {
        self.marks.get(name)
    }

    /// Return all node specs belonging to the given group.
    pub fn nodes_in_group(&self, group: &str) -> Vec<&NodeSpec> {
        self.nodes
            .values()
            .filter(|n| n.group.as_deref() == Some(group))
            .collect()
    }

    /// Find the first node spec whose `html_tag` matches the given tag name.
    pub fn node_by_html_tag(&self, tag: &str) -> Option<&NodeSpec> {
        self.nodes
            .values()
            .find(|n| n.html_tag.as_deref() == Some(tag))
    }

    /// Iterate over all node specs.
    pub fn all_nodes(&self) -> impl Iterator<Item = &NodeSpec> {
        self.nodes.values()
    }

    /// Iterate over all mark specs.
    pub fn all_marks(&self) -> impl Iterator<Item = &MarkSpec> {
        self.marks.values()
    }

    /// Return the list of mark names that can be toggled at the given node.
    ///
    /// Rules:
    /// 1. Active marks are always included (so the user can toggle them off).
    /// 2. Only nodes whose content expression includes `inline` or `text` allow
    ///    marks at all.
    /// 3. A candidate mark is excluded if any active mark's `excludes` field
    ///    covers it, or if the candidate's own `excludes` field covers any
    ///    active mark.
    pub fn allowed_marks_at(
        &self,
        node_spec: &NodeSpec,
        active_mark_names: &[&str],
    ) -> Vec<String> {
        let mut result = Vec::new();
        let allows_inline = node_spec
            .content
            .parts
            .iter()
            .any(|p| p.group == "inline" || p.group == "text");

        for mark_spec in self.all_marks() {
            let is_active = active_mark_names.contains(&mark_spec.name.as_str());

            // Active marks are always toggleable (so they can be removed).
            if is_active {
                result.push(mark_spec.name.clone());
                continue;
            }

            // Non-inline nodes don't support marks.
            if !allows_inline {
                continue;
            }

            // Check if any active mark excludes this candidate.
            let excluded_by_active = active_mark_names.iter().any(|&active_name| {
                if let Some(active_spec) = self.mark(active_name) {
                    mark_excluded_by(&active_spec.excludes, &mark_spec.name)
                } else {
                    false
                }
            });
            if excluded_by_active {
                continue;
            }

            // Check if this candidate excludes any active mark.
            let excludes_active = active_mark_names
                .iter()
                .any(|&active_name| mark_excluded_by(&mark_spec.excludes, active_name));
            if excludes_active {
                continue;
            }

            result.push(mark_spec.name.clone());
        }
        result
    }

    /// Return node type names that can be inserted at the given parent, assuming
    /// `existing_child_count` children already exist.
    pub fn insertable_nodes_at(
        &self,
        parent_spec: &NodeSpec,
        existing_child_count: usize,
    ) -> Vec<String> {
        let mut result = Vec::new();
        let mut remaining = existing_child_count;

        let mut accepting_groups: Vec<&str> = Vec::new();

        for part in &parent_spec.content.parts {
            let min = part.min as usize;
            let max = part.max.map(|m| m as usize);

            if remaining >= min {
                let consumed = match max {
                    Some(m) => remaining.min(m),
                    None => remaining,
                };
                remaining = remaining.saturating_sub(consumed);

                let at_max = max.map(|m| consumed >= m).unwrap_or(false);
                if !at_max {
                    accepting_groups.push(&part.group);
                }
            } else {
                // Mandatory part unsatisfied — only it is accepting, stop here
                accepting_groups.push(&part.group);
                break;
            }
        }

        let excluded_roles = |role: &NodeRole| -> bool {
            matches!(
                role,
                NodeRole::Doc
                    | NodeRole::Text
                    | NodeRole::ListItem
                    | NodeRole::TextBlock
                    | NodeRole::HardBreak
                    | NodeRole::Inline
            )
        };

        for node_spec in self.all_nodes() {
            if excluded_roles(&node_spec.role) {
                continue;
            }
            let matches = accepting_groups
                .iter()
                .any(|&group| node_spec.name == group || node_spec.group.as_deref() == Some(group));
            if matches {
                result.push(node_spec.name.clone());
            }
        }

        result
    }

    /// Build a schema from a JSON object.
    ///
    /// Expected format (matches the TypeScript SchemaDefinition type):
    /// ```json
    /// {
    ///   "nodes": [{ "name": "paragraph", "content": "inline*", "group": "block", "role": "textBlock", "htmlTag": "p" }, ...],
    ///   "marks": [{ "name": "bold" }, ...]
    /// }
    /// ```
    pub fn from_json(value: &serde_json::Value) -> Result<Self, String> {
        let nodes_arr = value
            .get("nodes")
            .and_then(|v| v.as_array())
            .ok_or_else(|| "schema JSON missing 'nodes' array".to_string())?;

        let marks_arr = value
            .get("marks")
            .and_then(|v| v.as_array())
            .unwrap_or(&Vec::new())
            .clone();

        let mut nodes = Vec::new();
        for node_val in nodes_arr {
            let name = node_val
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "node spec missing 'name'".to_string())?
                .to_string();

            let content_str = node_val
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            let content = ContentRule::parse(content_str)
                .map_err(|e| format!("content rule parse error for {name}: {e}"))?;

            let group = node_val
                .get("group")
                .and_then(|v| v.as_str())
                .map(String::from);
            let html_tag = node_val
                .get("htmlTag")
                .and_then(|v| v.as_str())
                .map(String::from);
            let is_void = node_val
                .get("isVoid")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            let role_str = node_val
                .get("role")
                .and_then(|v| v.as_str())
                .unwrap_or("block");

            let role = match role_str {
                "doc" => NodeRole::Doc,
                "textBlock" => NodeRole::TextBlock,
                "list" => {
                    let ordered = name.contains("ordered") || name.contains("Ordered");
                    NodeRole::List { ordered }
                }
                "listItem" => NodeRole::ListItem,
                "text" => NodeRole::Text,
                "hardBreak" => NodeRole::HardBreak,
                "inline" => NodeRole::Inline,
                _ => NodeRole::Block,
            };

            let mut attrs = HashMap::new();
            if let Some(attrs_obj) = node_val.get("attrs").and_then(|v| v.as_object()) {
                for (attr_name, attr_val) in attrs_obj {
                    attrs.insert(
                        attr_name.clone(),
                        AttrSpec {
                            default: attr_val.get("default").cloned(),
                        },
                    );
                }
            }

            nodes.push(NodeSpec {
                name,
                content,
                group,
                attrs,
                role,
                html_tag,
                is_void,
            });
        }

        let mut marks = Vec::new();
        for mark_val in &marks_arr {
            let name = mark_val
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "mark spec missing 'name'".to_string())?
                .to_string();

            let mut attrs = HashMap::new();
            if let Some(attrs_obj) = mark_val.get("attrs").and_then(|v| v.as_object()) {
                for (attr_name, attr_val) in attrs_obj {
                    attrs.insert(
                        attr_name.clone(),
                        AttrSpec {
                            default: attr_val.get("default").cloned(),
                        },
                    );
                }
            }

            let excludes = mark_val
                .get("excludes")
                .and_then(|v| v.as_str())
                .map(String::from);

            marks.push(MarkSpec {
                name,
                attrs,
                excludes,
            });
        }

        Ok(Schema::new(nodes, marks))
    }
}

/// Check whether an `excludes` field covers a given mark name.
///
/// - `None` → no exclusions.
/// - `Some("_")` → excludes all marks.
/// - Otherwise, space-separated list of mark names.
fn mark_excluded_by(excludes: &Option<String>, mark_name: &str) -> bool {
    match excludes {
        None => false,
        Some(exc) => {
            if exc == "_" {
                return true;
            }
            exc.split_whitespace().any(|e| e == mark_name)
        }
    }
}
