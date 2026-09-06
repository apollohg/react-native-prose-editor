#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DocumentStats {
    pub node_count: usize,
    pub max_depth: usize,
}

/// Exact reusable evidence from a complete document-validation pass.
///
/// This stays separate from [`DocumentStats`] so adding internal admission
/// evidence does not change the public struct-literal contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DocumentValidationMetrics {
    pub(crate) metadata_bytes: usize,
    pub(crate) validation_work: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DocumentValidationReport {
    pub(crate) stats: DocumentStats,
    pub(crate) metrics: DocumentValidationMetrics,
}

pub struct DocumentValidator;

impl DocumentValidator {
    pub fn validate(
        doc: &Document,
        schema: &Schema,
        limits: &ResourceLimits,
    ) -> BoundaryResult<DocumentStats> {
        let work_limit = limits.max_document_nodes.saturating_mul(128);
        let budget = WorkBudget::new(work_limit);
        Self::validate_with_budget(doc, schema, limits, &budget, work_limit)
    }

    pub(crate) fn validate_report(
        doc: &Document,
        schema: &Schema,
        limits: &ResourceLimits,
    ) -> BoundaryResult<DocumentValidationReport> {
        let work_limit = limits.max_document_nodes.saturating_mul(128);
        let budget = WorkBudget::new(work_limit);
        Self::validate_report_with_budget(doc, schema, limits, &budget, work_limit)
    }

    pub(crate) fn validate_with_budget(
        doc: &Document,
        schema: &Schema,
        limits: &ResourceLimits,
        budget: &WorkBudget,
        work_limit: usize,
    ) -> BoundaryResult<DocumentStats> {
        Self::validate_report_with_budget(doc, schema, limits, budget, work_limit)
            .map(|report| report.stats)
    }

    fn validate_report_with_budget(
        doc: &Document,
        schema: &Schema,
        limits: &ResourceLimits,
        budget: &WorkBudget,
        work_limit: usize,
    ) -> BoundaryResult<DocumentValidationReport> {
        #[cfg(test)]
        crate::yrs_engine::observability::record_document_validation();
        let work_before = budget.consumed(work_limit);
        let root_spec = schema.node(doc.root().node_type()).ok_or_else(|| {
            BoundaryError::new("DOCUMENT_INVALID", "document root is not in the schema")
        })?;
        if doc.root().node_type() != schema.doc_node_type()
            || !doc.root().is_element()
            || !matches!(root_spec.role, NodeRole::Doc)
        {
            return Err(BoundaryError::new(
                "DOCUMENT_INVALID",
                format!(
                    "document root '{}' does not have the doc role",
                    doc.root().node_type()
                ),
            ));
        }

        let mut state = DocumentValidationState {
            stats: DocumentStats {
                node_count: 0,
                max_depth: 0,
            },
            metadata_meter: JsonValueMeter::new(
                limits.max_input_bytes,
                work_limit,
                limits.max_document_depth,
                0,
            ),
        };
        validate_node(
            doc.root(),
            schema,
            limits,
            1,
            &mut state,
            budget,
            work_limit,
        )?;
        Ok(DocumentValidationReport {
            stats: state.stats,
            metrics: DocumentValidationMetrics {
                metadata_bytes: state.metadata_meter.bytes(),
                validation_work: budget.consumed(work_limit).saturating_sub(work_before),
            },
        })
    }
}

/// Validate an incoming mark set before canonical schema-rank ordering.
///
/// Yjs has one value per attribute key, so same-type duplicates are ambiguous.
/// Valid input order is otherwise irrelevant because semantic previews
/// canonicalize it after applying the operation.
pub(crate) fn validate_input_mark_set(marks: &[Mark], schema: &Schema) -> BoundaryResult<()> {
    validate_mark_set(marks, schema, false).map(|_| ())
}

/// Whether a validated mark set was already in schema-rank order.
///
/// Rank order is the one non-canonical property of an incoming mark set that
/// [`canonicalize_yrs_document`] repairs by sorting. Duplicate same-type
/// marks, unknown marks, and invalid attributes stay fatal: no sort makes
/// them representable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MarkSetOrder {
    Canonical,
    NeedsCanonicalization,
}

fn validate_mark_set(
    marks: &[Mark],
    schema: &Schema,
    require_canonical_order: bool,
) -> BoundaryResult<MarkSetOrder> {
    let work_limit = marks.len().saturating_mul(128).max(128);
    let budget = WorkBudget::new(work_limit);
    let mut seen = (marks.len() > 8).then(|| {
        #[cfg(test)]
        record_mark_set_hash_allocation();
        HashSet::with_capacity(marks.len())
    });
    let mut previous_rank = None;
    let mut order = MarkSetOrder::Canonical;
    for (index, mark) in marks.iter().enumerate() {
        let duplicate = match &mut seen {
            Some(seen) => !seen.insert(mark.mark_type()),
            None => marks[..index]
                .iter()
                .any(|previous| previous.mark_type() == mark.mark_type()),
        };
        if duplicate {
            let mut error = BoundaryError::new(
                "DOCUMENT_INVALID",
                "duplicate same-type marks cannot be represented by standard Yjs attributes",
            );
            error.details = Some(serde_json::json!({
                "field": "marks",
                "markType": mark.mark_type(),
                "reason": "duplicateType",
            }));
            return Err(error);
        }
        let rank = schema.mark_rank(mark.mark_type()).ok_or_else(|| {
            BoundaryError::new(
                "UNKNOWN_MARK",
                format!("unknown mark '{}'", mark.mark_type()),
            )
        })?;
        if previous_rank.is_some_and(|previous| rank < previous) {
            if require_canonical_order {
                let mut error = BoundaryError::new(
                    "DOCUMENT_INVALID",
                    "mark order does not match ProseMirror schema rank",
                );
                error.details = Some(serde_json::json!({
                    "field": "marks",
                    "reason": "nonCanonicalOrder",
                }));
                return Err(error);
            }
            order = MarkSetOrder::NeedsCanonicalization;
        }
        let spec = schema
            .mark(mark.mark_type())
            .expect("ranked mark must have a schema spec");
        validate_attrs(
            mark.attrs(),
            &spec.attrs,
            spec.allow_undeclared_attrs,
            mark.mark_type(),
            &budget,
            work_limit,
        )?;
        previous_rank = Some(rank);
    }
    Ok(order)
}

/// Validate canonical mark representation throughout an immutable document.
pub(crate) fn validate_canonical_marks(document: &Document, schema: &Schema) -> BoundaryResult<()> {
    validate_canonical_marks_with_evidence(document, schema).map(|_| ())
}

pub(crate) fn validate_canonical_marks_with_evidence<'schema>(
    document: &Document,
    schema: &'schema Schema,
) -> BoundaryResult<CanonicalMarksEvidence<'schema>> {
    validate_marks_with_evidence(document, schema, true)
}

/// As [`validate_canonical_marks_with_evidence`], but for a document being
/// admitted from outside the engine.
///
/// An importable document is not required to arrive in schema-rank mark
/// order. `<em><strong>x</strong></em>` and `<strong><em>x</em></strong>` are
/// the same document, and a serialized ProseMirror doc preserves whatever
/// order its producer applied. Non-canonical order is therefore reported as
/// evidence, which makes the caller canonicalize — the same sort every step
/// already performs — rather than rejecting content the engine is about to
/// repair anyway. Everything that no sort can fix stays fatal.
pub(crate) fn validate_importable_marks_with_evidence<'schema>(
    document: &Document,
    schema: &'schema Schema,
) -> BoundaryResult<CanonicalMarksEvidence<'schema>> {
    validate_marks_with_evidence(document, schema, false)
}

fn validate_marks_with_evidence<'schema>(
    document: &Document,
    schema: &'schema Schema,
    require_canonical_order: bool,
) -> BoundaryResult<CanonicalMarksEvidence<'schema>> {
    fn visit(root: &Node, schema: &Schema, require_canonical_order: bool) -> BoundaryResult<bool> {
        let mut pending = vec![root];
        let mut is_canonical = true;
        while let Some(node) = pending.pop() {
            #[cfg(test)]
            crate::yrs_engine::observability::record_canonical_mark_node_visited();
            if node.is_text()
                && validate_mark_set(node.marks(), schema, require_canonical_order)?
                    == MarkSetOrder::NeedsCanonicalization
            {
                is_canonical = false;
            }
            if let Some(content) = node.content() {
                let mut previous = None;
                for child in content.iter() {
                    if child.text_str().is_some_and(str::is_empty)
                        || previous.is_some_and(|previous: &Node| {
                            previous.is_text()
                                && child.is_text()
                                && super::steps::marks_eq(previous.marks(), child.marks())
                        })
                    {
                        is_canonical = false;
                    }
                    previous = Some(child);
                }
                pending.extend(content.iter().rev());
            }
        }
        Ok(is_canonical)
    }

    #[cfg(test)]
    crate::yrs_engine::observability::record_canonical_mark_validation_attempt();
    let result = visit(document.root(), schema, require_canonical_order).map(|is_canonical| {
        CanonicalMarksEvidence {
            source_root: document.root().clone(),
            source_schema: schema,
            is_canonical,
        }
    });
    #[cfg(test)]
    if result.is_ok() {
        crate::yrs_engine::observability::record_canonical_mark_validation_completion();
    }
    result
}

struct DocumentValidationState {
    stats: DocumentStats,
    metadata_meter: JsonValueMeter,
}

fn validate_node(
    node: &Node,
    schema: &Schema,
    limits: &ResourceLimits,
    depth: usize,
    state: &mut DocumentValidationState,
    budget: &WorkBudget,
    work_limit: usize,
) -> BoundaryResult<()> {
    let mut pending = vec![(node, depth)];
    while let Some((node, depth)) = pending.pop() {
        consume_document_work(budget, work_limit, 1)?;
        state.stats.node_count = state.stats.node_count.saturating_add(1);
        state.stats.max_depth = state.stats.max_depth.max(depth);
        if state.stats.node_count > limits.max_document_nodes {
            return Err(BoundaryError::limit(
                "DOCUMENT_LIMIT_EXCEEDED",
                limits.max_document_nodes,
                state.stats.node_count,
            ));
        }
        if depth > limits.max_document_depth {
            return Err(BoundaryError::limit(
                "DOCUMENT_LIMIT_EXCEEDED",
                limits.max_document_depth,
                depth,
            ));
        }

        if node.is_text() {
            if schema.node(node.node_type()).is_none() {
                return Err(BoundaryError::new(
                    "DOCUMENT_INVALID",
                    "text node is not in the schema",
                ));
            }
            validate_marks(node, schema, budget, work_limit)?;
            continue;
        }

        if node.node_type() == "__opaque" || node.node_type() == "__opaque_json" {
            validate_opaque(node, schema, budget, work_limit, &mut state.metadata_meter)?;
            continue;
        }

        let spec = schema.node(node.node_type()).ok_or_else(|| {
            BoundaryError::new(
                "DOCUMENT_INVALID",
                format!("unknown node '{}'", node.node_type()),
            )
        })?;
        validate_attrs(
            node.attrs(),
            &spec.attrs,
            spec.allow_undeclared_attrs,
            node.node_type(),
            budget,
            work_limit,
        )?;

        if node.is_void() {
            continue;
        }

        let content = node.content().ok_or_else(|| {
            BoundaryError::new("DOCUMENT_INVALID", "non-void schema node has no content")
        })?;
        let children = content.children();
        let matches = spec
            .content
            .matches_with_budget(
                children,
                |child, symbol| child_matches_group(child, symbol, schema),
                budget,
            )
            .map_err(|()| {
                let mut error = BoundaryError::limit(
                    "DOCUMENT_LIMIT_EXCEEDED",
                    work_limit,
                    work_limit.saturating_add(1),
                );
                error.details = Some(serde_json::json!({ "phase": "documentWork" }));
                error
            })?;
        if !matches {
            let child_types = children
                .iter()
                .map(|child| child.node_type())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(BoundaryError::new(
                "DOCUMENT_INVALID",
                format!(
                    "node '{}' content [{}] does not match its content expression",
                    node.node_type(),
                    child_types
                ),
            ));
        }

        let child_depth = depth.saturating_add(1);
        pending.extend(content.iter().rev().map(|child| (child, child_depth)));
    }
    Ok(())
}

fn validate_attrs(
    attrs: &HashMap<String, serde_json::Value>,
    specs: &HashMap<String, crate::schema::AttrSpec>,
    allow_undeclared: bool,
    owner: &str,
    budget: &WorkBudget,
    work_limit: usize,
) -> BoundaryResult<()> {
    for (name, spec) in specs {
        consume_document_work(budget, work_limit, 1)?;
        if let Some(value) = attrs.get(name).or(spec.default.as_ref()) {
            spec.validate_value(value).map_err(|message| {
                BoundaryError::new(
                    "DOCUMENT_INVALID",
                    format!("'{owner}' attribute '{name}': {message}"),
                )
            })?;
        }
        if !spec.has_default && !attrs.contains_key(name) {
            return Err(BoundaryError::new(
                "REQUIRED_ATTRIBUTE_MISSING",
                format!("'{owner}' requires attribute '{name}'"),
            ));
        }
    }
    for name in attrs.keys() {
        consume_document_work(budget, work_limit, 1)?;
        if !allow_undeclared && !specs.contains_key(name) {
            return Err(BoundaryError::new(
                "DOCUMENT_INVALID",
                format!("'{owner}' contains undeclared attribute '{name}'"),
            ));
        }
    }
    Ok(())
}

fn validate_marks(
    node: &Node,
    schema: &Schema,
    budget: &WorkBudget,
    work_limit: usize,
) -> BoundaryResult<()> {
    for mark in node.marks() {
        consume_document_work(budget, work_limit, 1)?;
        let spec = schema.mark(mark.mark_type()).ok_or_else(|| {
            BoundaryError::new(
                "UNKNOWN_MARK",
                format!("unknown mark '{}'", mark.mark_type()),
            )
        })?;
        validate_attrs(
            mark.attrs(),
            &spec.attrs,
            spec.allow_undeclared_attrs,
            mark.mark_type(),
            budget,
            work_limit,
        )?;
    }
    Ok(())
}

fn consume_document_work(
    budget: &WorkBudget,
    work_limit: usize,
    amount: usize,
) -> BoundaryResult<()> {
    if budget.consume_n(amount) {
        return Ok(());
    }
    let mut error = BoundaryError::limit(
        "DOCUMENT_LIMIT_EXCEEDED",
        work_limit,
        work_limit.saturating_add(1),
    );
    error.details = Some(serde_json::json!({ "phase": "documentWork" }));
    Err(error)
}

fn validate_opaque(
    node: &Node,
    schema: &Schema,
    budget: &WorkBudget,
    work_limit: usize,
    metadata_meter: &mut JsonValueMeter,
) -> BoundaryResult<()> {
    metadata_meter
        .admit_object(node.attrs())
        .map_err(map_opaque_metadata_limit)?;
    if !node.is_void() {
        return Err(BoundaryError::new(
            "DOCUMENT_INVALID",
            "opaque sentinel nodes must be void",
        ));
    }
    let placement = node
        .attrs()
        .get("opaque_placement")
        .and_then(|value| value.as_str());
    if !matches!(placement, Some("block" | "inline")) {
        return Err(BoundaryError::new(
            "DOCUMENT_INVALID",
            "opaque node is missing a valid placement",
        ));
    }
    if node.node_type() == "__opaque_json" {
        const KEYS: &[&str] = &["opaque_placement", "original_json", "original_type"];
        if node.attrs().len() != KEYS.len()
            || node.attrs().keys().any(|key| !KEYS.contains(&key.as_str()))
        {
            return Err(BoundaryError::new(
                "DOCUMENT_INVALID",
                "opaque JSON node has non-canonical metadata",
            ));
        }
        let original_type = node
            .attrs()
            .get("original_type")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                BoundaryError::new("DOCUMENT_INVALID", "opaque JSON original_type is invalid")
            })?;
        if matches!(original_type, "__opaque" | "__opaque_json" | "__skip")
            || schema.node(original_type).is_some()
        {
            return Err(BoundaryError::new(
                "DOCUMENT_INVALID",
                "opaque JSON original_type must remain unknown and non-reserved",
            ));
        }
        let original = node
            .attrs()
            .get("original_json")
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| {
                BoundaryError::new("DOCUMENT_INVALID", "opaque JSON payload must be an object")
            })?;
        if original.get("type").and_then(serde_json::Value::as_str) != Some(original_type) {
            return Err(BoundaryError::new(
                "DOCUMENT_INVALID",
                "opaque JSON payload type does not match original_type",
            ));
        }
        let empty_attrs = serde_json::Map::new();
        let original_attrs = original
            .get("attrs")
            .and_then(serde_json::Value::as_object)
            .unwrap_or(&empty_attrs);
        let normalized_type =
            crate::serialize::normalized_wire_json_node_type(original_type, original_attrs);
        if schema
            .node_for_json(original_type, Some(original_attrs))
            .is_some()
            || schema.node(&normalized_type).is_some()
        {
            return Err(BoundaryError::new(
                "DOCUMENT_INVALID",
                "opaque JSON payload normalizes to a known schema node",
            ));
        }
        return Ok(());
    }

    const HTML_KEYS: &[&str] = &[
        "html_tag",
        "opaque_placement",
        "html_attrs",
        "text_content",
        "inner_html",
    ];
    if node
        .attrs()
        .keys()
        .any(|key| !HTML_KEYS.contains(&key.as_str()))
    {
        return Err(BoundaryError::new(
            "DOCUMENT_INVALID",
            "opaque HTML node has non-canonical metadata",
        ));
    }
    let tag = node
        .attrs()
        .get("html_tag")
        .and_then(serde_json::Value::as_str)
        .filter(|tag| crate::schema::is_safe_html_tag(tag))
        .ok_or_else(|| BoundaryError::new("DOCUMENT_INVALID", "opaque HTML tag is invalid"))?;
    if let Some(attrs) = node.attrs().get("html_attrs") {
        let attrs = attrs.as_object().ok_or_else(|| {
            BoundaryError::new("DOCUMENT_INVALID", "opaque HTML attrs must be an object")
        })?;
        let mut folded_keys = HashSet::with_capacity(attrs.len());
        for (key, value) in attrs {
            consume_document_work(budget, work_limit, 1)?;
            // HTML attribute names are ASCII case-insensitive. Accept only
            // lowercase HTML keys or html5ever's exact SVG/MathML adjusted
            // spellings; arbitrary mixed-case private keys could otherwise
            // change meaning after export/reimport. Reject rather than fold so
            // duplicate keys cannot collapse ambiguously.
            if !crate::serialize::html_in::opaque_html_attr_has_canonical_case(tag, key)
                || !crate::schema::is_safe_html_attr(key)
                || value.as_str().is_none()
                || !folded_keys.insert(key.to_ascii_lowercase())
            {
                return Err(BoundaryError::new(
                    "DOCUMENT_INVALID",
                    "opaque HTML attribute metadata is invalid",
                ));
            }
        }
        if !crate::serialize::html_in::opaque_html_metadata_remains_opaque(
            tag,
            attrs,
            placement.expect("opaque placement was validated above"),
            schema,
        ) {
            return Err(BoundaryError::new(
                "DOCUMENT_INVALID",
                "opaque HTML metadata would reparse as a known semantic node or mark",
            ));
        }
    } else if !crate::serialize::html_in::opaque_html_metadata_remains_opaque(
        tag,
        &serde_json::Map::new(),
        placement.expect("opaque placement was validated above"),
        schema,
    ) {
        return Err(BoundaryError::new(
            "DOCUMENT_INVALID",
            "opaque HTML tag would reparse as a known semantic node or mark",
        ));
    }
    for key in ["text_content", "inner_html"] {
        if node
            .attrs()
            .get(key)
            .is_some_and(|value| value.as_str().is_none())
        {
            return Err(BoundaryError::new(
                "DOCUMENT_INVALID",
                "opaque HTML text metadata must be strings",
            ));
        }
    }
    Ok(())
}

fn map_opaque_metadata_limit(error: JsonMeterError) -> BoundaryError {
    let field = match error.dimension {
        JsonMeterDimension::Bytes => "maxInputBytes",
        JsonMeterDimension::Work => "documentWork",
        JsonMeterDimension::Depth => "maxDocumentDepth",
    };
    let mut boundary = BoundaryError::limit("DOCUMENT_LIMIT_EXCEEDED", error.limit, error.actual);
    boundary.details = Some(serde_json::json!({
        "phase": "opaqueMetadata",
        "field": field,
    }));
    boundary
}

/// Check if a child node matches a content group name.
///
/// A child matches if:
/// - Its node_type equals the group name exactly, OR
/// - Its schema spec belongs to the named group
fn child_matches_group(child: &Node, group: &str, schema: &Schema) -> bool {
    if matches!(child.node_type(), "__opaque" | "__opaque_json") {
        let placement = child
            .attrs()
            .get("opaque_placement")
            .and_then(|value| value.as_str());
        return placement
            .is_some_and(|placement| schema.symbol_accepts_opaque_placement(group, placement));
    }
    schema.node_matches_symbol(child.node_type(), group)
}

#[cfg(test)]
#[path = "../apply_document_validation_tests.rs"]
mod document_validation_stats_tests;
