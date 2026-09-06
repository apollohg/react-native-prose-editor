impl Schema {
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

    pub(crate) fn hard_break_node_types(&self) -> impl Iterator<Item = &str> {
        self.nodes.values().filter_map(|spec| {
            matches!(spec.role, NodeRole::HardBreak).then_some(spec.name.as_str())
        })
    }

    /// Look up a mark spec by name.
    pub fn mark(&self, name: &str) -> Option<&MarkSpec> {
        self.marks.get(name)
    }

    /// Return the ProseMirror schema rank for a mark type.
    pub fn mark_rank(&self, name: &str) -> Option<usize> {
        self.mark_order
            .iter()
            .position(|candidate| candidate == name)
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

    // Not reachable from production call paths after the Task 16C legacy runtime
    // removal; exercised by crate tests.
    #[allow(dead_code)]
    pub fn text_node_type(&self) -> &str {
        &self.text_node_name
    }

    /// Return all node specs belonging to the given group.
    // Not reachable from production call paths after the Task 16C legacy runtime
    // removal; exercised by crate tests.
    #[allow(dead_code)]
    pub fn nodes_in_group(&self, group: &str) -> Vec<&NodeSpec> {
        self.groups
            .get(group)
            .into_iter()
            .flatten()
            .filter_map(|name| self.nodes.get(name))
            .collect()
    }

    pub(crate) fn node_matches_symbol(&self, node_type: &str, symbol: &str) -> bool {
        node_type == symbol
            || self.groups.get(symbol).is_some_and(|members| {
                members
                    .binary_search_by(|member| member.as_str().cmp(node_type))
                    .is_ok()
            })
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

    pub(crate) fn nodes_with_html_rules_for_tag(&self, tag: &str) -> &[String] {
        self.html_rules_by_tag
            .get(tag)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn node_for_json(
        &self,
        node_type: &str,
        attrs: Option<&serde_json::Map<String, serde_json::Value>>,
    ) -> Option<&NodeSpec> {
        if let Some(spec) = self.node(node_type) {
            return Some(spec);
        }
        self.json_node_types
            .get(node_type)?
            .iter()
            .find_map(|name| {
                let spec = self.node(name)?;
                let projection = spec.json_projection.as_ref()?;
                projection
                    .attrs
                    .iter()
                    .all(|(name, value)| {
                        attrs
                            .and_then(|attrs| attrs.get(name))
                            .is_some_and(|actual| json_projection_values_equal(actual, value))
                    })
                    .then_some(spec)
            })
    }

    pub(crate) fn projected_nodes_for_json<'a>(
        &'a self,
        node_type: &str,
    ) -> impl Iterator<Item = &'a NodeSpec> + 'a {
        self.json_node_types
            .get(node_type)
            .into_iter()
            .flatten()
            .filter_map(|name| self.nodes.get(name))
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
        self.node_order
            .iter()
            .filter_map(|name| self.nodes.get(name))
    }

    /// Iterate over all mark specs.
    pub fn all_marks(&self) -> impl Iterator<Item = &MarkSpec> {
        self.mark_order
            .iter()
            .filter_map(|name| self.marks.get(name))
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
    // Not reachable from production call paths after the Task 16C legacy runtime
    // removal; exercised by crate tests.
    #[allow(dead_code)]
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
                |child_type, symbol| self.node_matches_symbol(child_type, symbol),
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
}
