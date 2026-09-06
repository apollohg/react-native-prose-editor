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
            consume_schema_work(budget, 1, "schema node index work budget exceeded")?;
            if let Some(tag) = node.html_tag.as_deref() {
                if !is_safe_html_tag(tag) {
                    return Err(SchemaValidationError::semantic(format!(
                        "node '{}' has invalid HTML tag '{}'",
                        node.name, tag
                    )));
                }
            }
            if let Some(rules) = &node.html_rules {
                if !is_safe_atom_html_tag(&rules.tag) {
                    return Err(SchemaValidationError::semantic(format!(
                        "node '{}' has invalid atom HTML tag '{}'",
                        node.name, rules.tag
                    )));
                }
                if rules.static_attrs.is_empty() {
                    return Err(SchemaValidationError::semantic(format!(
                        "node '{}' atom HTML rules require a static attribute discriminator",
                        node.name
                    )));
                }
                if let Some(name) = rules
                    .static_attrs
                    .iter()
                    .map(|(name, _)| name)
                    .chain(rules.attr_map.iter().map(|(_, name)| name))
                    .find(|name| !is_safe_atom_html_attr(name))
                {
                    return Err(SchemaValidationError::semantic(format!(
                        "node '{}' has invalid atom HTML attribute '{}'",
                        node.name, name
                    )));
                }
                if !node.is_void || node.allow_undeclared_attrs {
                    return Err(SchemaValidationError::semantic(format!(
                        "node '{}' atom HTML rules require a void node with declared attributes",
                        node.name
                    )));
                }
                let mapped_attrs = rules
                    .attr_map
                    .iter()
                    .map(|(name, _)| name.as_str())
                    .collect::<HashSet<_>>();
                if mapped_attrs.len() != rules.attr_map.len()
                    || mapped_attrs.len() != node.attrs.len()
                    || node
                        .attrs
                        .keys()
                        .any(|name| !mapped_attrs.contains(name.as_str()))
                {
                    return Err(SchemaValidationError::semantic(format!(
                        "node '{}' atom HTML attrMap must cover every declared attribute exactly once",
                        node.name
                    )));
                }
                let targets = rules
                    .attr_map
                    .iter()
                    .map(|(_, target)| target.as_str())
                    .collect::<HashSet<_>>();
                if targets.len() != rules.attr_map.len() {
                    return Err(SchemaValidationError::semantic(format!(
                        "node '{}' atom HTML attrMap targets must be unique",
                        node.name
                    )));
                }
                if rules
                    .static_attrs
                    .iter()
                    .any(|(name, _)| targets.contains(name.as_str()))
                {
                    return Err(SchemaValidationError::semantic(format!(
                        "node '{}' atom HTML attrs collide with a static discriminator",
                        node.name
                    )));
                }
            }
            if let Some(name) = node.attrs.keys().find(|name| !is_safe_html_attr(name)) {
                return Err(SchemaValidationError::semantic(format!(
                    "node '{}' has invalid HTML attribute identifier '{}'",
                    node.name, name
                )));
            }
            if node.attrs.values().any(|attr| {
                attr.has_default != attr.default.is_some() || attr.validate_definition().is_err()
            }) {
                return Err(SchemaValidationError::semantic(format!(
                    "node '{}' has an inconsistent attribute default",
                    node.name
                )));
            }
            if let Some(projection) = &node.json_projection {
                if projection.node_type.is_empty()
                    || matches!(
                        projection.node_type.as_str(),
                        "__opaque" | "__opaque_json" | "__skip"
                    )
                    || projection.attrs.keys().any(|name| !is_safe_html_attr(name))
                    || projection.attrs.values().any(|value| {
                        !matches!(
                            value,
                            serde_json::Value::Null
                                | serde_json::Value::Bool(_)
                                | serde_json::Value::Number(_)
                                | serde_json::Value::String(_)
                        )
                    })
                {
                    return Err(SchemaValidationError::semantic(format!(
                        "node '{}' has an invalid JSON projection",
                        node.name
                    )));
                }
            }
            if !node_names.insert(node.name.clone()) {
                return Err(SchemaValidationError::semantic(format!(
                    "duplicate node name '{}'",
                    node.name
                )));
            }
        }
        let html_rules_nodes = nodes
            .iter()
            .filter(|node| node.html_rules.is_some())
            .collect::<Vec<_>>();
        for (index, node) in html_rules_nodes.iter().enumerate() {
            let rules = node.html_rules.as_ref().expect("html-rules node");
            for previous in &html_rules_nodes[..index] {
                let previous_rules = previous.html_rules.as_ref().expect("html-rules node");
                if rules.tag != previous_rules.tag {
                    continue;
                }
                consume_schema_work(budget, 1, "schema atom HTML ambiguity work budget exceeded")?;
                let conflicts = rules.static_attrs.iter().any(|(name, value)| {
                    previous_rules
                        .static_attrs
                        .iter()
                        .find(|(previous_name, _)| previous_name == name)
                        .is_some_and(|(_, previous_value)| previous_value != value)
                });
                if !conflicts {
                    return Err(SchemaValidationError::semantic(format!(
                        "node '{}' has ambiguous atom HTML rules",
                        node.name
                    )));
                }
            }
        }
        let projected_nodes = nodes
            .iter()
            .filter(|node| node.json_projection.is_some())
            .collect::<Vec<_>>();
        for (index, node) in projected_nodes.iter().enumerate() {
            let projection = node.json_projection.as_ref().expect("projected node");
            if node_names.contains(&projection.node_type) {
                return Err(SchemaValidationError::semantic(format!(
                    "node '{}' projects to native node name '{}'",
                    node.name, projection.node_type
                )));
            }
            if let Some(legacy_name) = legacy_heading_projection_name(projection) {
                if node.name != legacy_name && node_names.contains(&legacy_name) {
                    return Err(SchemaValidationError::semantic(format!(
                        "node '{}' JSON projection conflicts with legacy heading alias '{}'",
                        node.name, legacy_name
                    )));
                }
            }
            if projection
                .attrs
                .keys()
                .any(|name| node.attrs.contains_key(name))
            {
                return Err(SchemaValidationError::semantic(format!(
                    "node '{}' JSON projection overlaps its native attributes",
                    node.name
                )));
            }
            for previous in &projected_nodes[..index] {
                consume_schema_work(
                    budget,
                    1,
                    "schema JSON projection ambiguity work budget exceeded",
                )?;
                let previous_projection =
                    previous.json_projection.as_ref().expect("projected node");
                if projection.node_type != previous_projection.node_type {
                    continue;
                }
                let overlaps = projection.attrs.iter().all(|(name, value)| {
                    previous_projection
                        .attrs
                        .get(name)
                        .is_none_or(|previous| json_projection_values_equal(value, previous))
                });
                if overlaps {
                    return Err(SchemaValidationError::semantic(format!(
                        "node '{}' has an ambiguous JSON projection",
                        node.name
                    )));
                }
            }
        }
        let mut mark_names = HashSet::new();
        for mark in &marks {
            consume_schema_work(budget, 1, "schema mark index work budget exceeded")?;
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
            if mark.attrs.values().any(|attr| {
                attr.has_default != attr.default.is_some() || attr.validate_definition().is_err()
            }) {
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
                    consume_schema_work(budget, 1, "schema group index work budget exceeded")?;
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
            consume_schema_work(budget, 1, "schema role index work budget exceeded")?;
            let mask = match node.role {
                NodeRole::Text | NodeRole::Inline | NodeRole::HardBreak => OPAQUE_INLINE_ROLE,
                NodeRole::Doc => 0,
                _ => OPAQUE_BLOCK_ROLE,
            };
            *symbol_role_masks.entry(node.name.clone()).or_insert(0) |= mask;
            if let Some(node_groups) = &node.group {
                for group in node_groups.split_whitespace() {
                    consume_schema_work(budget, 1, "schema role index work budget exceeded")?;
                    *symbol_role_masks.entry(group.to_string()).or_insert(0) |= mask;
                }
            }
        }

        let mut node_html_tags = HashMap::new();
        let mut html_rules_by_tag: HashMap<String, Vec<String>> = HashMap::new();
        let mut json_node_types: HashMap<String, Vec<String>> = HashMap::new();
        for node in &nodes {
            if let Some(tag) = &node.html_tag {
                consume_schema_work(budget, 1, "schema HTML index work budget exceeded")?;
                // Several supported schemas intentionally map multiple semantic
                // node types to the same HTML tag (for example task and bullet
                // lists). Preserve descriptor order as the deterministic import
                // precedence while keeping lookup constant-time.
                node_html_tags
                    .entry(tag.clone())
                    .or_insert_with(|| node.name.clone());
            }
            if let Some(rules) = &node.html_rules {
                consume_schema_work(budget, 1, "schema atom HTML index work budget exceeded")?;
                html_rules_by_tag
                    .entry(rules.tag.clone())
                    .or_default()
                    .push(node.name.clone());
            }
            if let Some(projection) = &node.json_projection {
                consume_schema_work(
                    budget,
                    1,
                    "schema JSON projection index work budget exceeded",
                )?;
                json_node_types
                    .entry(projection.node_type.clone())
                    .or_default()
                    .push(node.name.clone());
            }
        }
        let mut mark_html_tags = HashMap::new();
        for mark in &marks {
            if let Some(tag) = &mark.html_tag {
                consume_schema_work(budget, 1, "schema HTML index work budget exceeded")?;
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

        let node_order = nodes.iter().map(|node| node.name.clone()).collect();
        let mark_order = marks.iter().map(|mark| mark.name.clone()).collect();
        let schema = Self {
            nodes: nodes
                .into_iter()
                .map(|node| (node.name.clone(), node))
                .collect(),
            node_order,
            marks: marks
                .into_iter()
                .map(|mark| (mark.name.clone(), mark))
                .collect(),
            mark_order,
            node_html_tags,
            html_rules_by_tag,
            json_node_types,
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
}
