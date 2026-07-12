pub mod content_rule;
pub mod presets;

use std::cell::Cell;
use std::collections::{HashMap, HashSet, VecDeque};

use crate::boundary::{BoundaryError, BoundaryResult, ResourceLimits};
use crate::model::{Document, Fragment, Node};
use crate::schema::content_rule::{
    ContentRule, ContentRuleError, WorkBudget, DEFAULT_RUNTIME_WORK_LIMIT,
};

#[derive(Debug)]
enum SchemaValidationError {
    Semantic(String),
    ResourceExhausted(String),
}

impl SchemaValidationError {
    fn semantic(message: impl Into<String>) -> Self {
        Self::Semantic(message.into())
    }

    fn resource(message: impl Into<String>) -> Self {
        Self::ResourceExhausted(message.into())
    }

    fn message(self) -> String {
        match self {
            Self::Semantic(message) | Self::ResourceExhausted(message) => message,
        }
    }
}

/// A schema defines the set of node types and mark types available in a document.
///
/// Node and mark names are plain strings, allowing the same schema structure to
/// support different naming conventions (e.g. camelCase for Tiptap, snake_case
/// for ProseMirror).
#[derive(Debug, Clone)]
pub struct Schema {
    nodes: HashMap<String, NodeSpec>,
    marks: HashMap<String, MarkSpec>,
    node_html_tags: HashMap<String, String>,
    mark_html_tags: HashMap<String, String>,
    preferred_text_block_name: Option<String>,
    fallback_list_item_name: Option<String>,
    groups: HashMap<String, Vec<String>>,
    symbol_role_masks: HashMap<String, u8>,
    doc_node_name: String,
    text_node_name: String,
}

const OPAQUE_INLINE_ROLE: u8 = 1;
const OPAQUE_BLOCK_ROLE: u8 = 2;

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
    /// Optional validated HTML tag used for importing and exporting this mark.
    pub html_tag: Option<String>,
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
    pub has_default: bool,
}

const DEFAULT_DOCUMENT_MAX_DEPTH: usize = 128;
const DEFAULT_DOCUMENT_MAX_NODES: usize = 10_000;
const DEFAULT_DOCUMENT_MAX_WORK: usize = 10_000;

struct DefaultConstructionBudget {
    work: Cell<usize>,
    nodes: Cell<usize>,
}

impl DefaultConstructionBudget {
    fn consume_work(&self) -> bool {
        if self.work.get() >= DEFAULT_DOCUMENT_MAX_WORK {
            return false;
        }
        self.work.set(self.work.get() + 1);
        true
    }
}

impl Schema {
    /// Create a schema from lists of node and mark specs.
    pub fn new(nodes: Vec<NodeSpec>, marks: Vec<MarkSpec>) -> Self {
        Self::try_new(nodes, marks).expect("invalid schema")
    }

    /// Create and validate a schema, returning a descriptive error for invalid
    /// role, name, content-symbol, or constructibility definitions.
    pub fn try_new(nodes: Vec<NodeSpec>, marks: Vec<MarkSpec>) -> Result<Self, String> {
        Self::try_new_with_budget(nodes, marks, &WorkBudget::new(usize::MAX))
            .map_err(SchemaValidationError::message)
    }

    fn try_new_with_budget(
        nodes: Vec<NodeSpec>,
        marks: Vec<MarkSpec>,
        budget: &WorkBudget,
    ) -> Result<Self, SchemaValidationError> {
        let mut node_names = HashSet::new();
        for node in &nodes {
            if let Some(tag) = node.html_tag.as_deref() {
                if !is_safe_html_tag(tag) {
                    return Err(SchemaValidationError::semantic(format!(
                        "node '{}' has invalid HTML tag '{}'",
                        node.name, tag
                    )));
                }
            }
            if let Some(name) = node.attrs.keys().find(|name| !is_safe_html_attr(name)) {
                return Err(SchemaValidationError::semantic(format!(
                    "node '{}' has invalid HTML attribute identifier '{}'",
                    node.name, name
                )));
            }
            if node
                .attrs
                .values()
                .any(|attr| attr.has_default != attr.default.is_some())
            {
                return Err(SchemaValidationError::semantic(format!(
                    "node '{}' has an inconsistent attribute default",
                    node.name
                )));
            }
            if !node_names.insert(node.name.clone()) {
                return Err(SchemaValidationError::semantic(format!(
                    "duplicate node name '{}'",
                    node.name
                )));
            }
        }
        let mut mark_names = HashSet::new();
        for mark in &marks {
            if let Some(tag) = mark.html_tag.as_deref() {
                const ALLOWED_MARK_TAGS: &[&str] = &[
                    "span", "strong", "em", "u", "s", "code", "a", "sub", "sup", "mark",
                ];
                if !ALLOWED_MARK_TAGS.contains(&tag) {
                    return Err(SchemaValidationError::semantic(format!(
                        "mark '{}' has disallowed HTML tag '{}'",
                        mark.name, tag
                    )));
                }
            }
            if let Some(name) = mark.attrs.keys().find(|name| !is_safe_html_attr(name)) {
                return Err(SchemaValidationError::semantic(format!(
                    "mark '{}' has invalid HTML attribute identifier '{}'",
                    mark.name, name
                )));
            }
            if mark
                .attrs
                .values()
                .any(|attr| attr.has_default != attr.default.is_some())
            {
                return Err(SchemaValidationError::semantic(format!(
                    "mark '{}' has an inconsistent attribute default",
                    mark.name
                )));
            }
            if !mark_names.insert(mark.name.clone()) {
                return Err(SchemaValidationError::semantic(format!(
                    "duplicate mark name '{}'",
                    mark.name
                )));
            }
        }

        let doc_names = nodes
            .iter()
            .filter(|node| matches!(node.role, NodeRole::Doc))
            .map(|node| node.name.clone())
            .collect::<Vec<_>>();
        let text_names = nodes
            .iter()
            .filter(|node| matches!(node.role, NodeRole::Text))
            .map(|node| node.name.clone())
            .collect::<Vec<_>>();
        if doc_names.len() != 1 {
            return Err(SchemaValidationError::semantic(format!(
                "schema must define exactly one doc role, found {}",
                doc_names.len()
            )));
        }
        if text_names.len() != 1 {
            return Err(SchemaValidationError::semantic(format!(
                "schema must define exactly one text role, found {}",
                text_names.len()
            )));
        }

        let mut groups: HashMap<String, Vec<String>> = HashMap::new();
        for node in &nodes {
            if let Some(node_groups) = &node.group {
                for group in node_groups.split_whitespace() {
                    groups
                        .entry(group.to_string())
                        .or_default()
                        .push(node.name.clone());
                }
            }
        }
        for names in groups.values_mut() {
            names.sort();
            names.dedup();
        }

        let mut symbol_role_masks = HashMap::new();
        for node in &nodes {
            let mask = match node.role {
                NodeRole::Text | NodeRole::Inline | NodeRole::HardBreak => OPAQUE_INLINE_ROLE,
                NodeRole::Doc => 0,
                _ => OPAQUE_BLOCK_ROLE,
            };
            *symbol_role_masks.entry(node.name.clone()).or_insert(0) |= mask;
            if let Some(node_groups) = &node.group {
                for group in node_groups.split_whitespace() {
                    *symbol_role_masks.entry(group.to_string()).or_insert(0) |= mask;
                }
            }
        }

        let mut node_html_tags = HashMap::new();
        for node in &nodes {
            if let Some(tag) = &node.html_tag {
                // Several supported schemas intentionally map multiple semantic
                // node types to the same HTML tag (for example task and bullet
                // lists). Preserve descriptor order as the deterministic import
                // precedence while keeping lookup constant-time.
                node_html_tags
                    .entry(tag.clone())
                    .or_insert_with(|| node.name.clone());
            }
        }
        let mut mark_html_tags = HashMap::new();
        for mark in &marks {
            if let Some(tag) = &mark.html_tag {
                mark_html_tags
                    .entry(tag.clone())
                    .or_insert_with(|| mark.name.clone());
            }
        }

        let preferred_text_block_name = nodes
            .iter()
            .filter(|node| matches!(node.role, NodeRole::TextBlock))
            .filter(|node| node.attrs.values().all(|attr| attr.has_default))
            .min_by_key(|node| {
                (
                    if node.html_tag.as_deref() == Some("p") || node.name == "paragraph" {
                        0
                    } else {
                        1
                    },
                    node.name.as_str(),
                )
            })
            .map(|node| node.name.clone());
        let fallback_list_item_name = nodes
            .iter()
            .filter(|node| matches!(node.role, NodeRole::ListItem))
            .min_by_key(|node| node.name.as_str())
            .map(|node| node.name.clone());

        let schema = Self {
            nodes: nodes
                .into_iter()
                .map(|node| (node.name.clone(), node))
                .collect(),
            marks: marks
                .into_iter()
                .map(|mark| (mark.name.clone(), mark))
                .collect(),
            node_html_tags,
            mark_html_tags,
            preferred_text_block_name,
            fallback_list_item_name,
            groups,
            symbol_role_masks,
            doc_node_name: doc_names.into_iter().next().expect("one doc role"),
            text_node_name: text_names.into_iter().next().expect("one text role"),
        };
        schema.validate_constructibility(budget)?;
        schema
            .default_document()
            .map_err(SchemaValidationError::semantic)?;
        Ok(schema)
    }

    fn validate_constructibility(&self, budget: &WorkBudget) -> Result<(), SchemaValidationError> {
        let mut dependents_by_symbol: HashMap<String, Vec<String>> = HashMap::new();
        for node in self.nodes.values() {
            for symbol in node.content.symbols() {
                if !self.nodes.contains_key(symbol) && !self.groups.contains_key(symbol) {
                    return Err(SchemaValidationError::semantic(format!(
                        "content rule for '{}' references unresolved node or group '{}'",
                        node.name, symbol
                    )));
                }
                if !budget.consume() {
                    return Err(SchemaValidationError::resource(
                        "schema constructibility work budget exceeded",
                    ));
                }
                dependents_by_symbol
                    .entry(symbol.to_string())
                    .or_default()
                    .push(node.name.clone());
            }
        }

        let eligible = self
            .nodes
            .values()
            .filter(|node| {
                !matches!(node.role, NodeRole::Text)
                    && !node.attrs.values().any(|attr| !attr.has_default)
            })
            .map(|node| node.name.clone())
            .collect::<HashSet<_>>();
        let mut generatable = HashSet::new();
        let mut constructible_symbols = HashSet::new();
        let mut queued = eligible.clone();
        let mut pending = eligible.iter().cloned().collect::<VecDeque<_>>();

        while let Some(name) = pending.pop_front() {
            queued.remove(&name);
            if generatable.contains(&name) {
                continue;
            }
            let node = self.nodes.get(&name).expect("indexed node");
            let constructible = node
                .content
                .is_constructible_with_budget(
                    |symbol| constructible_symbols.contains(symbol),
                    budget,
                )
                .map_err(|()| {
                    SchemaValidationError::resource("schema constructibility work budget exceeded")
                })?;
            if constructible {
                generatable.insert(name.clone());
                let node = self.nodes.get(&name).expect("indexed node");
                let symbols = std::iter::once(name.as_str()).chain(
                    node.group
                        .as_deref()
                        .into_iter()
                        .flat_map(str::split_whitespace),
                );
                for symbol in symbols {
                    if !constructible_symbols.insert(symbol.to_string()) {
                        continue;
                    }
                    if let Some(nodes) = dependents_by_symbol.get(symbol) {
                        for dependent in nodes {
                            if !budget.consume() {
                                return Err(SchemaValidationError::resource(
                                    "schema constructibility work budget exceeded",
                                ));
                            }
                            if eligible.contains(dependent)
                                && !generatable.contains(dependent)
                                && queued.insert(dependent.clone())
                            {
                                pending.push_back(dependent.clone());
                            }
                        }
                    }
                }
            }
        }

        for node in self.nodes.values() {
            let constructible = node
                .content
                .is_constructible_with_budget(
                    |symbol| constructible_symbols.contains(symbol),
                    budget,
                )
                .map_err(|()| {
                    SchemaValidationError::resource("schema constructibility work budget exceeded")
                })?;
            if !constructible {
                return Err(SchemaValidationError::semantic(format!(
                    "content rule for '{}' has required content that cannot be auto-created",
                    node.name
                )));
            }
        }
        Ok(())
    }

    fn candidate_names_for_symbol<'a>(&'a self, symbol: &'a str) -> impl Iterator<Item = &'a str> {
        self.nodes
            .get_key_value(symbol)
            .map(|(name, _)| name.as_str())
            .into_iter()
            .chain(
                self.groups
                    .get(symbol)
                    .into_iter()
                    .flatten()
                    .map(String::as_str),
            )
    }

    fn candidates_for_symbol<'a>(&'a self, symbol: &'a str) -> impl Iterator<Item = &'a NodeSpec> {
        self.candidate_names_for_symbol(symbol)
            .filter_map(|name| self.nodes.get(name))
    }

    /// Look up a node spec by name.
    pub fn node(&self, name: &str) -> Option<&NodeSpec> {
        self.nodes.get(name)
    }

    /// Look up a mark spec by name.
    pub fn mark(&self, name: &str) -> Option<&MarkSpec> {
        self.marks.get(name)
    }

    pub fn symbol_accepts_opaque_placement(&self, symbol: &str, placement: &str) -> bool {
        let mask = self.symbol_role_masks.get(symbol).copied().unwrap_or(0);
        match placement {
            "inline" => mask & OPAQUE_INLINE_ROLE != 0,
            "block" => mask & OPAQUE_BLOCK_ROLE != 0,
            _ => false,
        }
    }

    pub fn doc_node_type(&self) -> &str {
        &self.doc_node_name
    }

    pub fn text_node_type(&self) -> &str {
        &self.text_node_name
    }

    /// Return all node specs belonging to the given group.
    pub fn nodes_in_group(&self, group: &str) -> Vec<&NodeSpec> {
        self.groups
            .get(group)
            .into_iter()
            .flatten()
            .filter_map(|name| self.nodes.get(name))
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
        self.list_item_type_for_with_budget(list_type, &WorkBudget::new(DEFAULT_DOCUMENT_MAX_WORK))
            .ok()
            .flatten()
    }

    pub(crate) fn list_item_type_for_with_budget(
        &self,
        list_type: &str,
        budget: &WorkBudget,
    ) -> Result<Option<String>, ()> {
        let Some(list_spec) = self.node(list_type) else {
            return Ok(None);
        };
        let initial_symbols = list_spec.content.initial_symbols_with_budget(budget)?;
        let mut candidates = Vec::new();
        for symbol in initial_symbols {
            for spec in self.candidates_for_symbol(symbol) {
                if !budget.consume() {
                    return Err(());
                }
                if matches!(spec.role, NodeRole::ListItem) {
                    candidates.push(spec);
                }
            }
        }
        candidates.sort_by(|a, b| a.name.cmp(&b.name));
        candidates.dedup_by(|a, b| a.name == b.name);

        let is_task_list = list_type.to_ascii_lowercase().contains("task");
        let is_task_item = |spec: &NodeSpec| {
            spec.name.to_ascii_lowercase().contains("task") || spec.attrs.contains_key("checked")
        };

        Ok(candidates
            .iter()
            .find(|spec| is_task_item(spec) == is_task_list)
            .or_else(|| candidates.first())
            .map(|spec| spec.name.clone()))
    }

    /// Find the first node spec whose `html_tag` matches the given tag name.
    pub fn node_by_html_tag(&self, tag: &str) -> Option<&NodeSpec> {
        self.node_html_tags
            .get(tag)
            .and_then(|name| self.nodes.get(name))
    }

    pub fn mark_by_html_tag(&self, tag: &str) -> Option<&MarkSpec> {
        self.mark_html_tags
            .get(tag)
            .and_then(|name| self.marks.get(name))
    }

    pub fn preferred_text_block(&self) -> Option<&NodeSpec> {
        self.preferred_text_block_name
            .as_deref()
            .and_then(|name| self.node(name))
    }

    pub fn fallback_list_item_type(&self) -> Option<&str> {
        self.fallback_list_item_name.as_deref()
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
        prefix_child_types: &[&str],
        suffix_child_types: &[&str],
    ) -> Vec<String> {
        self.insertable_nodes_at_with_budget(
            parent_spec,
            prefix_child_types,
            suffix_child_types,
            &WorkBudget::new(DEFAULT_RUNTIME_WORK_LIMIT),
        )
        .unwrap_or_default()
    }

    pub(crate) fn insertable_nodes_at_with_budget(
        &self,
        parent_spec: &NodeSpec,
        prefix_child_types: &[&str],
        suffix_child_types: &[&str],
        budget: &WorkBudget,
    ) -> Result<Vec<String>, ()> {
        let mut result = Vec::new();

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
            if !budget.consume() {
                return Err(());
            }
            if excluded_roles(&node_spec.role) {
                continue;
            }
            let candidate_types = prefix_child_types
                .iter()
                .copied()
                .chain(std::iter::once(node_spec.name.as_str()))
                .chain(suffix_child_types.iter().copied())
                .collect::<Vec<_>>();
            if parent_spec.content.matches_with_budget(
                &candidate_types,
                |child_type, symbol| {
                    self.node(child_type)
                        .is_some_and(|spec| node_spec_matches_symbol(spec, symbol))
                },
                budget,
            )? {
                result.push(node_spec.name.clone());
            }
        }

        Ok(result)
    }

    /// Construct the shortest complete document accepted by the schema using
    /// only nodes whose attributes have defaults. Text nodes are never created
    /// implicitly because they require text content.
    pub fn default_document(&self) -> Result<Document, String> {
        let doc_spec = self
            .node(&self.doc_node_name)
            .ok_or_else(|| "schema has no doc role".to_string())?;
        let root = self
            .construct_default_node(
                doc_spec,
                &mut HashSet::new(),
                0,
                &DefaultConstructionBudget {
                    work: Cell::new(0),
                    nodes: Cell::new(0),
                },
            )
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
        depth: usize,
        budget: &DefaultConstructionBudget,
    ) -> Option<Node> {
        if depth > DEFAULT_DOCUMENT_MAX_DEPTH || !budget.consume_work() {
            return None;
        }
        if matches!(spec.role, NodeRole::Text)
            || spec.attrs.values().any(|attr| !attr.has_default)
            || !visiting.insert(spec.name.clone())
        {
            return None;
        }

        let children = spec.content.minimal_match_with(
            |symbol| {
                let mut candidates = Vec::new();
                for candidate in self.candidates_for_symbol(symbol) {
                    if !budget.consume_work() {
                        return None;
                    }
                    candidates.push(candidate);
                }
                for _ in &candidates {
                    if !budget.consume_work() {
                        return None;
                    }
                }
                candidates.sort_by_key(|candidate| default_node_priority(candidate));
                candidates.into_iter().find_map(|candidate| {
                    self.construct_default_node(candidate, visiting, depth + 1, budget)
                })
            },
            || budget.consume_work(),
        );
        visiting.remove(&spec.name);
        let children = children?;
        if budget.nodes.get() >= DEFAULT_DOCUMENT_MAX_NODES {
            return None;
        }
        budget.nodes.set(budget.nodes.get() + 1);
        let attrs = spec
            .attrs
            .iter()
            .filter(|(_, attr)| attr.has_default)
            .map(|(name, attr)| {
                (
                    name.clone(),
                    attr.default.clone().expect("validated explicit default"),
                )
            })
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
        Self::from_json_with_limits(value, &ResourceLimits::default())
            .map_err(|error| error.message)
    }

    pub fn from_json_with_limits(
        value: &serde_json::Value,
        limits: &ResourceLimits,
    ) -> BoundaryResult<Self> {
        let nodes_arr = value
            .get("nodes")
            .and_then(|v| v.as_array())
            .ok_or_else(|| {
                BoundaryError::new("SCHEMA_INVALID", "schema JSON missing 'nodes' array")
            })?;

        if nodes_arr.len() > limits.max_schema_nodes {
            return Err(BoundaryError::limit(
                "SCHEMA_INVALID",
                limits.max_schema_nodes,
                nodes_arr.len(),
            ));
        }

        let expression_bytes = nodes_arr.iter().try_fold(0usize, |total, node| {
            total.checked_add(
                node.get("content")
                    .and_then(serde_json::Value::as_str)
                    .map_or(0, str::len),
            )
        });
        let expression_bytes = expression_bytes.ok_or_else(|| {
            BoundaryError::new("SCHEMA_INVALID", "schema expression size overflow")
        })?;
        if expression_bytes > limits.max_schema_expression_bytes {
            return Err(BoundaryError::limit(
                "SCHEMA_INVALID",
                limits.max_schema_expression_bytes,
                expression_bytes,
            ));
        }

        let work_limit = limits
            .max_schema_nodes
            .saturating_mul(64)
            .saturating_add(limits.max_schema_expression_bytes.saturating_mul(32));
        let budget = WorkBudget::new(work_limit);

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
                .ok_or_else(|| BoundaryError::new("SCHEMA_INVALID", "node spec missing 'name'"))?
                .to_string();

            let content_str = node_val
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            let content =
                ContentRule::parse_with_budget(content_str, &budget).map_err(|error| {
                    schema_boundary_error(content_rule_schema_error(&name, error), work_limit)
                })?;

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
                            has_default: attr_val.get("default").is_some(),
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
                .ok_or_else(|| BoundaryError::new("SCHEMA_INVALID", "mark spec missing 'name'"))?
                .to_string();

            let mut attrs = HashMap::new();
            if let Some(attrs_obj) = mark_val.get("attrs").and_then(|v| v.as_object()) {
                for (attr_name, attr_val) in attrs_obj {
                    attrs.insert(
                        attr_name.clone(),
                        AttrSpec {
                            default: attr_val.get("default").cloned(),
                            has_default: attr_val.get("default").is_some(),
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
                html_tag: mark_val
                    .get("htmlTag")
                    .and_then(|value| value.as_str())
                    .map(str::to_ascii_lowercase),
                attrs,
                excludes,
                allow_undeclared_attrs,
            });
        }

        Schema::try_new_with_budget(nodes, marks, &budget)
            .map_err(|error| schema_boundary_error(error, work_limit))
    }
}

fn is_safe_html_tag(tag: &str) -> bool {
    let mut chars = tag.chars();
    matches!(chars.next(), Some('a'..='z'))
        && chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
}

fn is_safe_html_attr(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(ch) if ch.is_ascii_alphabetic() || ch == '_' || ch == ':')
        && chars.all(|ch| {
            ch.is_ascii_alphanumeric() || matches!(ch, '_' | ':' | '.' | '-')
        })
}

fn content_rule_schema_error(name: &str, error: ContentRuleError) -> SchemaValidationError {
    match error {
        ContentRuleError::Semantic(message) => SchemaValidationError::semantic(format!(
            "content rule parse error for {name}: {message}"
        )),
        ContentRuleError::ResourceExhausted(message) => SchemaValidationError::resource(format!(
            "content rule parse error for {name}: {message}"
        )),
    }
}

fn schema_boundary_error(error: SchemaValidationError, work_limit: usize) -> BoundaryError {
    match error {
        SchemaValidationError::Semantic(message) => BoundaryError::new("SCHEMA_INVALID", message),
        SchemaValidationError::ResourceExhausted(message) => {
            let mut error =
                BoundaryError::limit("SCHEMA_INVALID", work_limit, work_limit.saturating_add(1));
            error.message = message;
            error.details = Some(serde_json::json!({ "phase": "schemaWork" }));
            error
        }
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
