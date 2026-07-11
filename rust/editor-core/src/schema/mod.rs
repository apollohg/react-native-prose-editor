pub mod content_rule;
pub mod presets;

use std::collections::{HashMap, HashSet};

use crate::model::{Document, Fragment, Node};
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
    /// If `true`, JSON ingestion (`set_json`/`insert_content_json`) admits attrs
    /// on this node that are not declared in `attrs`, instead of filtering them
    /// out. Default `false`. This is an opt-in escape hatch for node types with
    /// an intentional pass-through-metadata contract (e.g. the `mention` node,
    /// which carries arbitrary app-defined attrs such as `id`/`kind`). Every
    /// other node type is filtered to its schema-declared attrs, matching the
    /// HTML ingestion path (`extract_node_attrs`).
    pub allow_undeclared_attrs: bool,
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
    /// If `true`, JSON ingestion (`set_json`/`insert_content_json`) admits attrs
    /// on this mark that are not declared in `attrs`, instead of filtering them
    /// out. Default `false`. This is an opt-in escape hatch mirroring
    /// `NodeSpec::allow_undeclared_attrs` for mark types with an intentional
    /// pass-through-metadata contract. Every other mark type is filtered to
    /// its schema-declared attrs.
    pub allow_undeclared_attrs: bool,
}

/// Specification for a single attribute on a node or mark type.
#[derive(Debug, Clone)]
pub struct AttrSpec {
    pub default: Option<serde_json::Value>,
}

impl Schema {
    /// Create a schema from lists of node and mark specs.
    pub fn new(nodes: Vec<NodeSpec>, marks: Vec<MarkSpec>) -> Self {
        Self::try_new(nodes, marks).expect("invalid schema")
    }

    /// Create and validate a schema, returning a descriptive error for invalid
    /// role, name, content-symbol, or constructibility definitions.
    pub fn try_new(nodes: Vec<NodeSpec>, marks: Vec<MarkSpec>) -> Result<Self, String> {
        let mut node_names = HashSet::new();
        for node in &nodes {
            if !node_names.insert(node.name.clone()) {
                return Err(format!("duplicate node name '{}'", node.name));
            }
        }
        let mut mark_names = HashSet::new();
        for mark in &marks {
            if !mark_names.insert(mark.name.clone()) {
                return Err(format!("duplicate mark name '{}'", mark.name));
            }
        }

        let doc_count = nodes
            .iter()
            .filter(|node| matches!(node.role, NodeRole::Doc))
            .count();
        let text_count = nodes
            .iter()
            .filter(|node| matches!(node.role, NodeRole::Text))
            .count();
        if doc_count != 1 {
            return Err(format!(
                "schema must define exactly one doc role, found {doc_count}"
            ));
        }
        if text_count != 1 {
            return Err(format!(
                "schema must define exactly one text role, found {text_count}"
            ));
        }

        for node in &nodes {
            for symbol in node.content.symbols() {
                if !nodes
                    .iter()
                    .any(|candidate| node_spec_matches_symbol(candidate, symbol))
                {
                    return Err(format!(
                        "content rule for '{}' references unresolved node or group '{}'",
                        node.name, symbol
                    ));
                }
            }
        }

        let mut generatable = HashSet::new();
        loop {
            let before = generatable.len();
            for node in &nodes {
                let has_required_attrs = node.attrs.values().any(|attr| attr.default.is_none());
                if !matches!(node.role, NodeRole::Text)
                    && !has_required_attrs
                    && node.content.is_constructible_with(|symbol| {
                        nodes.iter().any(|candidate| {
                            generatable.contains(&candidate.name)
                                && node_spec_matches_symbol(candidate, symbol)
                        })
                    })
                {
                    generatable.insert(node.name.clone());
                }
            }
            if generatable.len() == before {
                break;
            }
        }
        if let Some(node) = nodes.iter().find(|node| {
            !node.content.is_constructible_with(|symbol| {
                nodes.iter().any(|candidate| {
                    generatable.contains(&candidate.name)
                        && node_spec_matches_symbol(candidate, symbol)
                })
            })
        }) {
            return Err(format!(
                "content rule for '{}' has required content that cannot be auto-created",
                node.name
            ));
        }

        Ok(Self {
            nodes: nodes
                .into_iter()
                .map(|node| (node.name.clone(), node))
                .collect(),
            marks: marks
                .into_iter()
                .map(|mark| (mark.name.clone(), mark))
                .collect(),
        })
    }

    /// Look up a node spec by name.
    pub fn node(&self, name: &str) -> Option<&NodeSpec> {
        self.nodes.get(name)
    }

    /// Look up a mark spec by name.
    pub fn mark(&self, name: &str) -> Option<&MarkSpec> {
        self.marks.get(name)
    }

    pub fn doc_node_type(&self) -> &str {
        self.nodes
            .values()
            .find(|node| matches!(node.role, NodeRole::Doc))
            .expect("validated schemas always contain one doc role")
            .name
            .as_str()
    }

    /// Return all node specs belonging to the given group.
    pub fn nodes_in_group(&self, group: &str) -> Vec<&NodeSpec> {
        self.nodes
            .values()
            .filter(|node| node_spec_matches_symbol(node, group) && node.name != group)
            .collect()
    }

    /// Node-type classification by schema role. These are the single source
    /// of truth for "is this a list / list item" — the renderer, position
    /// map, and undo inverse-step computation must all agree, so none of
    /// them may match node-type names directly.
    pub fn is_list(&self, node_type: &str) -> bool {
        self.node(node_type)
            .map(|spec| matches!(spec.role, NodeRole::List { .. }))
            .unwrap_or(false)
    }

    pub fn is_list_item(&self, node_type: &str) -> bool {
        self.node(node_type)
            .map(|spec| matches!(spec.role, NodeRole::ListItem))
            .unwrap_or(false)
    }

    pub fn is_ordered_list(&self, node_type: &str) -> bool {
        self.node(node_type)
            .map(|spec| matches!(spec.role, NodeRole::List { ordered: true }))
            .unwrap_or(false)
    }

    /// Resolve the item node type a list of `list_type` should wrap content in.
    ///
    /// Resolution: (1) the list's first content part named a node directly;
    /// (2) group expansion filtered to `NodeRole::ListItem`. Within a group,
    /// task lists prefer task-item candidates (name contains "task" or the
    /// spec declares a `checked` attr); non-task lists prefer non-task
    /// candidates. Ties resolve alphabetically for determinism.
    pub fn list_item_type_for(&self, list_type: &str) -> Option<String> {
        let list_spec = self.node(list_type)?;
        let initial_symbols = list_spec.content.initial_symbols();
        let mut candidates: Vec<&NodeSpec> = self
            .all_nodes()
            .filter(|spec| {
                initial_symbols
                    .iter()
                    .any(|symbol| node_spec_matches_symbol(spec, symbol))
            })
            .filter(|spec| matches!(spec.role, NodeRole::ListItem))
            .collect();
        candidates.sort_by(|a, b| a.name.cmp(&b.name));
        candidates.dedup_by(|a, b| a.name == b.name);

        let is_task_list = list_type.to_ascii_lowercase().contains("task");
        let is_task_item = |spec: &NodeSpec| {
            spec.name.to_ascii_lowercase().contains("task") || spec.attrs.contains_key("checked")
        };

        candidates
            .iter()
            .find(|spec| is_task_item(spec) == is_task_list)
            .or_else(|| candidates.first())
            .map(|spec| spec.name.clone())
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
            .symbols()
            .any(|symbol| symbol == "inline" || symbol == "text");

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
    /// `existing_child_types` is the actual prefix before the insertion point.
    pub fn insertable_nodes_at(
        &self,
        parent_spec: &NodeSpec,
        existing_child_types: &[&str],
    ) -> Vec<String> {
        let mut result = Vec::new();
        let accepting_groups = parent_spec.content.accepting_symbols_after(
            existing_child_types,
            |child_type, symbol| {
                self.node(child_type)
                    .is_some_and(|spec| node_spec_matches_symbol(spec, symbol))
            },
        );

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
                .any(|group| node_spec_matches_symbol(node_spec, group));
            if matches {
                result.push(node_spec.name.clone());
            }
        }

        result
    }

    /// Construct the shortest complete document accepted by the schema using
    /// only nodes whose attributes have defaults. Text nodes are never created
    /// implicitly because they require text content.
    pub fn default_document(&self) -> Result<Document, String> {
        let doc_spec = self
            .all_nodes()
            .find(|node| matches!(node.role, NodeRole::Doc))
            .ok_or_else(|| "schema has no doc role".to_string())?;
        let root = self
            .construct_default_node(doc_spec, &mut HashSet::new())
            .ok_or_else(|| {
                format!(
                    "schema cannot construct a default document for '{}'",
                    doc_spec.name
                )
            })?;
        Ok(Document::new(root))
    }

    fn construct_default_node(
        &self,
        spec: &NodeSpec,
        visiting: &mut HashSet<String>,
    ) -> Option<Node> {
        if matches!(spec.role, NodeRole::Text)
            || spec.attrs.values().any(|attr| attr.default.is_none())
            || !visiting.insert(spec.name.clone())
        {
            return None;
        }

        let children = spec.content.minimal_match_with(|symbol| {
            let mut candidates = self
                .all_nodes()
                .filter(|candidate| node_spec_matches_symbol(candidate, symbol))
                .collect::<Vec<_>>();
            candidates.sort_by_key(|candidate| default_node_priority(candidate));
            candidates
                .into_iter()
                .find_map(|candidate| self.construct_default_node(candidate, visiting))
        });
        visiting.remove(&spec.name);
        let children = children?;
        let attrs = spec
            .attrs
            .iter()
            .filter_map(|(name, attr)| attr.default.clone().map(|value| (name.clone(), value)))
            .collect();
        Some(if spec.is_void {
            Node::void(spec.name.clone(), attrs)
        } else {
            Node::element(spec.name.clone(), attrs, Fragment::from(children))
        })
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
            let allow_undeclared_attrs = node_val
                .get("allowUndeclaredAttrs")
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
                allow_undeclared_attrs,
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

            let allow_undeclared_attrs = mark_val
                .get("allowUndeclaredAttrs")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            marks.push(MarkSpec {
                name,
                attrs,
                excludes,
                allow_undeclared_attrs,
            });
        }

        Schema::try_new(nodes, marks)
    }
}

pub(crate) fn node_spec_matches_symbol(node: &NodeSpec, symbol: &str) -> bool {
    node.name == symbol
        || node
            .group
            .as_deref()
            .is_some_and(|groups| groups.split_whitespace().any(|group| group == symbol))
}

fn default_node_priority(node: &NodeSpec) -> (u8, &str) {
    let priority = match node.role {
        NodeRole::TextBlock
            if node.html_tag.as_deref() == Some("p") || node.name == "paragraph" =>
        {
            0
        }
        NodeRole::TextBlock => 1,
        _ => 2,
    };
    (priority, node.name.as_str())
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
