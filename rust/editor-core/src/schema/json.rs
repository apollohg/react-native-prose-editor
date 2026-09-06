impl Schema {
    /// Build a schema from a JSON object.
    ///
    /// Expected format (matches the TypeScript SchemaDefinition type):
    /// ```json
    /// {
    ///   "nodes": [{ "name": "paragraph", "content": "inline*", "group": "block", "role": "textBlock", "htmlTag": "p" }, ...],
    ///   "marks": [{ "name": "bold" }, ...]
    /// }
    /// ```
    // Not reachable from production call paths after the Task 16C legacy runtime
    // removal; exercised by crate tests.
    #[allow(dead_code)]
    pub fn from_json(value: &serde_json::Value) -> Result<Self, String> {
        Self::from_json_with_limits(value, &ResourceLimits::default())
            .map_err(|error| error.message)
    }

    pub fn from_json_with_limits(
        value: &serde_json::Value,
        limits: &ResourceLimits,
    ) -> BoundaryResult<Self> {
        #[cfg(test)]
        SCHEMA_FROM_JSON_COUNT.with(|count| count.set(count.get() + 1));

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

        let empty_marks = Vec::new();
        let marks_arr = value
            .get("marks")
            .and_then(|v| v.as_array())
            .unwrap_or(&empty_marks);

        let mut nodes = Vec::new();
        for node_val in nodes_arr {
            let name = node_val
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| BoundaryError::new("SCHEMA_INVALID", "node spec missing 'name'"))?;
            admit_schema_string(name, &budget, work_limit)?;

            let content_str = node_val
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            let content =
                ContentRule::parse_with_budget(content_str, &budget).map_err(|error| {
                    schema_boundary_error(content_rule_schema_error(name, error), work_limit)
                })?;

            if let Some(group) = node_val.get("group").and_then(|v| v.as_str()) {
                admit_schema_groups(group, &budget, work_limit)?;
            }
            admit_schema_attrs(node_val.get("attrs"), &budget, work_limit)?;

            let group = node_val
                .get("group")
                .and_then(|v| v.as_str())
                .map(String::from);
            let html_tag = node_val
                .get("htmlTag")
                .and_then(|v| v.as_str())
                .map(String::from);
            let html_rules = parse_html_rules(node_val.get("html"), &budget, work_limit)?;
            let json_projection = match node_val.get("json") {
                None => None,
                Some(value) => {
                    let value = value.as_object().ok_or_else(|| {
                        BoundaryError::new(
                            "SCHEMA_INVALID",
                            "node JSON projection must be an object",
                        )
                    })?;
                    let node_type = value
                        .get("type")
                        .and_then(serde_json::Value::as_str)
                        .filter(|node_type| !node_type.is_empty())
                        .ok_or_else(|| {
                            BoundaryError::new(
                                "SCHEMA_INVALID",
                                "node JSON projection missing 'type'",
                            )
                        })?;
                    admit_schema_string(node_type, &budget, work_limit)?;
                    let attrs = match value.get("attrs") {
                        None => HashMap::new(),
                        Some(attrs) => {
                            let attrs = attrs.as_object().ok_or_else(|| {
                                BoundaryError::new(
                                    "SCHEMA_INVALID",
                                    "node JSON projection attrs must be an object",
                                )
                            })?;
                            let mut admitted = HashMap::new();
                            for (name, value) in attrs {
                                admit_schema_string(name, &budget, work_limit)?;
                                if !matches!(
                                    value,
                                    serde_json::Value::Null
                                        | serde_json::Value::Bool(_)
                                        | serde_json::Value::Number(_)
                                        | serde_json::Value::String(_)
                                ) {
                                    return Err(BoundaryError::new(
                                        "SCHEMA_INVALID",
                                        "node JSON projection attrs must be scalar",
                                    ));
                                }
                                admit_schema_value(value, &budget, work_limit, 1)?;
                                admitted.insert(name.clone(), value.clone());
                            }
                            admitted
                        }
                    };
                    Some(NodeJsonProjection {
                        node_type: node_type.to_string(),
                        attrs,
                    })
                }
            };
            let is_void = node_val
                .get("isVoid")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let deletable_on_backspace = node_val
                .get("deletableOnBackspace")
                .and_then(|v| v.as_bool());
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
                    attrs.insert(attr_name.clone(), AttrSpec::from_json(attr_val)?);
                }
            }

            nodes.push(NodeSpec {
                name: name.to_string(),
                content,
                group,
                attrs,
                role,
                html_tag,
                html_rules,
                json_projection,
                is_void,
                deletable_on_backspace,
                allow_undeclared_attrs,
            });
        }

        let mut marks = Vec::new();
        for mark_val in marks_arr {
            consume_schema_boundary_work(&budget, 1, work_limit)?;
            let name = mark_val
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| BoundaryError::new("SCHEMA_INVALID", "mark spec missing 'name'"))?;
            admit_schema_string(name, &budget, work_limit)?;
            admit_schema_attrs(mark_val.get("attrs"), &budget, work_limit)?;

            let mut attrs = HashMap::new();
            if let Some(attrs_obj) = mark_val.get("attrs").and_then(|v| v.as_object()) {
                for (attr_name, attr_val) in attrs_obj {
                    attrs.insert(attr_name.clone(), AttrSpec::from_json(attr_val)?);
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
                name: name.to_string(),
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
