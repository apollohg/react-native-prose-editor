struct JsonMatchCursor<'a> {
    expected: Option<&'a [Value]>,
    index: usize,
    pending_text: Option<PendingJsonText<'a>>,
    previous_actual_marks: Option<Box<Attrs>>,
    actual_text_pending: bool,
}

struct JsonProjectionContext<'schema, 'limits> {
    schema: &'schema Schema,
    budget: ConversionBudget<'limits>,
}

struct PendingJsonText<'a> {
    text: Option<&'a str>,
    offset: usize,
}

impl<'a> JsonMatchCursor<'a> {
    fn new(expected: Option<&'a [Value]>) -> Self {
        Self {
            expected,
            index: 0,
            pending_text: None,
            previous_actual_marks: None,
            actual_text_pending: false,
        }
    }

    fn finish_pending(&mut self, matched: &mut bool) {
        if let Some(pending) = self.pending_text.take() {
            *matched &= pending
                .text
                .is_some_and(|text| pending.offset == text.len());
            self.index = self.index.saturating_add(1);
        }
        self.previous_actual_marks = None;
        self.actual_text_pending = false;
    }

    fn next_node(&mut self, matched: &mut bool) -> Option<&'a Value> {
        self.finish_pending(matched);
        let node = self.expected.and_then(|values| values.get(self.index));
        *matched &= node.is_some();
        self.index = self.index.saturating_add(1);
        node
    }

    fn observe_text(
        &mut self,
        text: &str,
        attrs: Option<Box<Attrs>>,
        schema: &Schema,
        depth: usize,
        budget: &mut ConversionBudget<'_>,
        matched: &mut bool,
    ) -> YrsEngineResult<()> {
        let coalesces = self.actual_text_pending
            && actual_marks_equal(self.previous_actual_marks.as_deref(), attrs.as_deref());
        if !coalesces {
            budget.admit_node(depth)?;
            self.finish_pending(matched);
            self.previous_actual_marks = attrs;
            self.actual_text_pending = true;
            if !*matched {
                return Ok(());
            }
            let node = self.expected.and_then(|values| values.get(self.index));
            let object = node.and_then(Value::as_object);
            let expected_text = object
                .and_then(|object| object.get("text"))
                .and_then(Value::as_str);
            let expected_marks = object
                .and_then(|object| object.get("marks"))
                .and_then(Value::as_array);
            let shape_matches = object.is_some_and(|object| {
                let marks_present = object.contains_key("marks");
                object.len() == if marks_present { 3 } else { 2 }
                    && object.get("type").and_then(Value::as_str) == Some("text")
                    && expected_text.is_some()
                    && (!marks_present || expected_marks.is_some_and(|marks| !marks.is_empty()))
            });
            *matched &= shape_matches
                && marks_match_json(
                    self.previous_actual_marks.as_deref(),
                    expected_marks,
                    schema,
                );
            if !*matched {
                return Ok(());
            }
            self.pending_text = Some(PendingJsonText {
                text: expected_text,
                offset: 0,
            });
        }
        if !*matched {
            return Ok(());
        }
        let pending = self
            .pending_text
            .as_mut()
            .expect("matching text is pending");
        let segment_matches = pending.text.is_some_and(|expected| {
            expected
                .get(pending.offset..)
                .is_some_and(|remaining| remaining.starts_with(text))
        });
        *matched &= segment_matches;
        if segment_matches {
            pending.offset = pending.offset.saturating_add(text.len());
        }
        Ok(())
    }

    fn finish(mut self, matched: &mut bool) -> usize {
        self.finish_pending(matched);
        *matched &= self.expected.map_or(0, <[Value]>::len) == self.index;
        self.index
    }
}

fn any_projection_equal(left: &Any, right: &Any) -> bool {
    if any_projects_null(left) && any_projects_null(right) {
        return true;
    }
    if matches!(left, Any::Number(_) | Any::BigInt(_))
        || matches!(right, Any::Number(_) | Any::BigInt(_))
    {
        return projected_number(left)
            .is_some_and(|left| projected_number(right).is_some_and(|right| left == right));
    }
    match (left, right) {
        (Any::Bool(left), Any::Bool(right)) => left == right,
        (Any::String(left), Any::String(right)) => left == right,
        (Any::Buffer(left), Any::Buffer(right)) => left == right,
        (Any::Buffer(left), Any::Array(right)) | (Any::Array(right), Any::Buffer(left)) => {
            left.len() == right.len()
                && left.iter().zip(right.iter()).all(|(left, right)| {
                    projected_number(right)
                        .is_some_and(|right| right == serde_json::Number::from(*left))
                })
        }
        (Any::Array(left), Any::Array(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .zip(right.iter())
                    .all(|(left, right)| any_projection_equal(left, right))
        }
        (Any::Map(left), Any::Map(right)) => {
            left.len() == right.len()
                && left.iter().all(|(key, left)| {
                    right
                        .get(key)
                        .is_some_and(|right| any_projection_equal(left, right))
                })
        }
        _ => false,
    }
}

fn projected_number(value: &Any) -> Option<serde_json::Number> {
    match value {
        Any::Number(value) => serde_json::Number::from_f64(*value),
        Any::BigInt(value) => Some((*value).into()),
        _ => None,
    }
}

fn any_projects_null(value: &Any) -> bool {
    matches!(value, Any::Null | Any::Undefined)
        || matches!(value, Any::Number(number) if !number.is_finite())
}

fn mark_value_omits_attrs(value: &Any) -> bool {
    matches!(value, Any::Bool(true)) || any_projects_null(value)
}

fn mark_value_projection_equal(left: &Any, right: &Any) -> bool {
    let left_omits_attrs = mark_value_omits_attrs(left);
    let right_omits_attrs = mark_value_omits_attrs(right);
    (left_omits_attrs && right_omits_attrs)
        || (!left_omits_attrs && !right_omits_attrs && any_projection_equal(left, right))
}

fn actual_marks_equal(left: Option<&Attrs>, right: Option<&Attrs>) -> bool {
    let left_len = left.map_or(0, Attrs::len);
    let right_len = right.map_or(0, Attrs::len);
    if left_len != right_len {
        return false;
    }
    if left_len == 0 {
        return true;
    }
    let (Some(left), Some(right)) = (left, right) else {
        return false;
    };
    left.iter().all(|(key, left)| {
        right
            .get(key)
            .is_some_and(|right| mark_value_projection_equal(left, right))
    })
}

fn validate_any_projection(
    value: &Any,
    budget: &mut ConversionBudget<'_>,
    depth: usize,
) -> YrsEngineResult<()> {
    budget.admit_any(depth, 1)?;
    match value {
        Any::Null | Any::Undefined => budget.charge_output(4),
        Any::Bool(value) => budget.charge_output(if *value { 4 } else { 5 }),
        Any::Number(value) => {
            let number = serde_json::Number::from_f64(*value);
            budget.charge_computed_output(|| {
                number.as_ref().map_or(4, |number| number.to_string().len())
            })
        }
        Any::BigInt(value) => budget.charge_computed_output(|| value.to_string().len()),
        Any::String(value) => budget.charge_computed_output(|| json_string_len(value)),
        Any::Buffer(value) => {
            budget.admit_any(depth, value.len())?;
            budget.charge_output(2usize.saturating_add(value.len().saturating_sub(1)))?;
            for byte in value.iter() {
                budget.charge_output(decimal_u8_len(*byte))?;
            }
            Ok(())
        }
        Any::Array(values) => {
            budget.charge_output(2usize.saturating_add(values.len().saturating_sub(1)))?;
            for value in values.iter() {
                validate_any_projection(value, budget, depth + 1)?;
            }
            Ok(())
        }
        Any::Map(values) => {
            budget.charge_output(2usize.saturating_add(values.len().saturating_sub(1)))?;
            for (key, value) in values.iter() {
                budget.charge_computed_output(|| json_string_len(key).saturating_add(1))?;
                validate_any_projection(value, budget, depth + 1)?;
            }
            Ok(())
        }
    }
}

fn any_matches_json(value: &Any, expected: Option<&Value>) -> bool {
    match value {
        Any::Null | Any::Undefined => expected.is_some_and(Value::is_null),
        Any::Bool(value) => expected.and_then(Value::as_bool) == Some(*value),
        Any::Number(value) => serde_json::Number::from_f64(*value).map_or_else(
            || expected.is_some_and(Value::is_null),
            |number| Some(&number) == expected.and_then(Value::as_number),
        ),
        Any::BigInt(value) => expected.and_then(Value::as_i64) == Some(*value),
        Any::String(value) => expected.and_then(Value::as_str) == Some(value.as_ref()),
        Any::Buffer(value) => expected.and_then(Value::as_array).is_some_and(|expected| {
            expected.len() == value.len()
                && value
                    .iter()
                    .zip(expected)
                    .all(|(byte, expected)| expected.as_u64() == Some(u64::from(*byte)))
        }),
        Any::Array(values) => expected.and_then(Value::as_array).is_some_and(|expected| {
            expected.len() == values.len()
                && values
                    .iter()
                    .zip(expected)
                    .all(|(value, expected)| any_matches_json(value, Some(expected)))
        }),
        Any::Map(values) => expected.and_then(Value::as_object).is_some_and(|expected| {
            expected.len() == values.len()
                && values
                    .iter()
                    .all(|(key, value)| any_matches_json(value, expected.get(key.as_str())))
        }),
    }
}

fn projection_value_matches_json(value: &Any, expected: &Value) -> bool {
    match value {
        Any::Number(value) if value.is_finite() => serde_json::Number::from_f64(*value)
            .is_some_and(|number| json_projection_values_equal(&Value::Number(number), expected)),
        Any::BigInt(value) => {
            json_projection_values_equal(&Value::Number((*value).into()), expected)
        }
        _ => any_matches_json(value, Some(expected)),
    }
}

fn element_matches_projection<T: ReadTxn>(
    element: &XmlElementRef,
    txn: &T,
    projection: &NodeJsonProjection,
) -> bool {
    projection.attrs.iter().all(|(name, expected)| {
        let Some(yrs::Out::Any(actual)) = element.get_attribute(txn, name) else {
            return false;
        };
        projection_value_matches_json(&actual, expected)
    })
}

fn resolve_wire_node_spec<'schema>(
    tag: &str,
    level: Option<u8>,
    schema: &'schema Schema,
    mut projection_matches: impl FnMut(&NodeJsonProjection) -> bool,
) -> Option<&'schema NodeSpec> {
    let (normalized, _) = normalized_type(tag, level);
    if normalized != tag {
        if let Some(spec) = schema.node(normalized) {
            return Some(spec);
        }
    }
    schema
        .projected_nodes_for_json(tag)
        .find(|spec| {
            spec.json_projection
                .as_ref()
                .is_some_and(&mut projection_matches)
        })
        .or_else(|| {
            if normalized == tag {
                schema.node(tag)
            } else {
                None
            }
        })
}

pub(crate) fn wire_element_node_spec<'schema, T: ReadTxn>(
    element: &XmlElementRef,
    txn: &T,
    schema: &'schema Schema,
) -> Option<&'schema NodeSpec> {
    resolve_wire_node_spec(
        element.tag().as_ref(),
        heading_level(element, txn),
        schema,
        |projection| element_matches_projection(element, txn, projection),
    )
}

pub(crate) fn prepared_wire_node_spec<'schema>(
    tag: &str,
    attrs: &[(String, Any)],
    schema: &'schema Schema,
) -> Option<&'schema NodeSpec> {
    let level = (tag == "heading")
        .then(|| {
            attrs
                .iter()
                .find(|(name, _)| name == "level")
                .and_then(|(_, value)| heading_level_from_any(value))
        })
        .flatten();
    resolve_wire_node_spec(tag, level, schema, |projection| {
        projection.attrs.iter().all(|(name, expected)| {
            attrs
                .iter()
                .find(|(candidate, _)| candidate == name)
                .is_some_and(|(_, actual)| projection_value_matches_json(actual, expected))
        })
    })
}

fn marks_match_json(attrs: Option<&Attrs>, expected: Option<&Vec<Value>>, schema: &Schema) -> bool {
    let attr_count = attrs.map_or(0, Attrs::len);
    let Some(expected) = expected else {
        return attr_count == 0;
    };
    if expected.len() != attr_count || expected.is_empty() {
        return false;
    }
    let Some(attrs) = attrs else {
        return false;
    };
    let mut previous: Option<(usize, &str)> = None;
    for mark in expected {
        let Some(object) = mark.as_object() else {
            return false;
        };
        let Some(name) = object.get("type").and_then(Value::as_str) else {
            return false;
        };
        let rank = schema.mark_rank(name).unwrap_or(usize::MAX);
        if previous.is_some_and(|(previous_rank, previous_name)| {
            (previous_rank, previous_name) >= (rank, name)
        }) {
            return false;
        }
        previous = Some((rank, name));
        let Some(value) = attrs.get(name) else {
            return false;
        };
        let omits_attrs = mark_value_omits_attrs(value);
        if object.len() != if omits_attrs { 1 } else { 2 }
            || (!omits_attrs && !any_matches_json(value, object.get("attrs")))
        {
            return false;
        }
    }
    true
}

fn heading_level<T: ReadTxn>(element: &XmlElementRef, txn: &T) -> Option<u8> {
    match element.get_attribute(txn, "level") {
        Some(yrs::Out::Any(value)) => heading_level_from_any(&value),
        _ => None,
    }
}

fn heading_level_from_any(value: &Any) -> Option<u8> {
    match value {
        Any::BigInt(value) => u8::try_from(*value).ok(),
        Any::Number(value) => (value.is_finite() && value.fract() == 0.0)
            .then(|| u8::try_from(*value as i64).ok())
            .flatten(),
        Any::String(value) => crate::serialize::parse_wire_heading_level_str(value),
        _ => None,
    }
    .filter(|level| (1..=6).contains(level))
}

fn normalized_type(tag: &str, level: Option<u8>) -> (&str, bool) {
    if tag != "heading" {
        return (tag, false);
    }
    match level {
        Some(1) => ("h1", true),
        Some(2) => ("h2", true),
        Some(3) => ("h3", true),
        Some(4) => ("h4", true),
        Some(5) => ("h5", true),
        Some(6) => ("h6", true),
        _ => (tag, false),
    }
}

fn match_xml_out_json<T: ReadTxn>(
    node: XmlOut,
    txn: &T,
    depth: usize,
    cursor: &mut JsonMatchCursor<'_>,
    matched: &mut bool,
    context: &mut JsonProjectionContext<'_, '_>,
    lookup: Option<&mut ImportLookupMaterializationCollector>,
) -> YrsEngineResult<()> {
    stacker::maybe_grow(
        RECURSION_RED_ZONE_BYTES,
        RECURSION_STACK_SEGMENT_BYTES,
        || match_xml_out_json_inner(node, txn, depth, cursor, matched, context, lookup),
    )
}

fn match_xml_out_json_inner<T: ReadTxn>(
    node: XmlOut,
    txn: &T,
    depth: usize,
    cursor: &mut JsonMatchCursor<'_>,
    matched: &mut bool,
    context: &mut JsonProjectionContext<'_, '_>,
    mut lookup: Option<&mut ImportLookupMaterializationCollector>,
) -> YrsEngineResult<()> {
    if lookup.as_deref().is_some_and(|lookup| lookup.has_failed()) {
        lookup = None;
    }
    match node {
        XmlOut::Element(element) => {
            match_xml_element_json(&element, txn, depth, cursor, matched, context, lookup)
        }
        XmlOut::Text(text) => {
            match_xml_text_json(&text, txn, depth, cursor, matched, context, lookup)
        }
        XmlOut::Fragment(fragment) => {
            context.budget.admit_traversal_depth(depth)?;
            if let Some(lookup) = lookup.as_deref_mut() {
                lookup.begin_fragment();
            }
            for child in fragment.children(txn) {
                match_xml_out_json(
                    child,
                    txn,
                    depth + 1,
                    cursor,
                    matched,
                    context,
                    lookup.as_deref_mut(),
                )?;
            }
            if let Some(lookup) = lookup {
                lookup.end_container();
            }
            Ok(())
        }
    }
}

fn match_xml_element_json<T: ReadTxn>(
    element: &XmlElementRef,
    txn: &T,
    depth: usize,
    cursor: &mut JsonMatchCursor<'_>,
    matched: &mut bool,
    context: &mut JsonProjectionContext<'_, '_>,
    mut lookup: Option<&mut ImportLookupMaterializationCollector>,
) -> YrsEngineResult<()> {
    if lookup.as_deref().is_some_and(|lookup| lookup.has_failed()) {
        lookup = None;
    }
    context.budget.admit_node(depth)?;
    // An element is always a projected-text coalescing boundary, including
    // after an earlier comparison mismatch. Keep actual resource accounting
    // independent from the sticky comparison result.
    let expected_node = cursor.next_node(matched);
    let expected_object = expected_node.and_then(Value::as_object);
    let expected_attrs = expected_object
        .and_then(|object| object.get("attrs"))
        .and_then(Value::as_object);
    let expected_content = expected_object
        .and_then(|object| object.get("content"))
        .and_then(Value::as_array)
        .map(Vec::as_slice);

    let tag = element.tag();
    let level = (tag.as_ref() == "heading")
        .then(|| heading_level(element, txn))
        .flatten();
    let (normalized_node_type, normalized_removes_level) = normalized_type(tag.as_ref(), level);
    let spec = wire_element_node_spec(element, txn, context.schema);
    let node_type = spec.map_or(normalized_node_type, |spec| spec.name.as_str());
    let removes_level = normalized_removes_level
        && node_type != tag.as_ref()
        && spec.is_none_or(|spec| !spec.attrs.contains_key("level"));
    let projection = spec.and_then(|spec| spec.json_projection.as_ref());
    let projected_type = projection.map_or(node_type, |projection| projection.node_type.as_str());
    let mut local_match = expected_object
        .is_some_and(|object| object.get("type").and_then(Value::as_str) == Some(projected_type));
    let collect_lookup = lookup.is_some();
    let mut lookup_attribute_work = ImportElementAttributeWork::new();
    let mut projected_attr_count = projection.map_or(0, |projection| projection.attrs.len());
    if let Some(projection) = projection {
        for (name, value) in &projection.attrs {
            local_match &= expected_attrs
                .and_then(|attrs| attrs.get(name))
                .is_some_and(|expected| json_projection_values_equal(expected, value));
        }
    }
    for (key, value) in element.attributes(txn) {
        let yrs::Out::Any(value) = value else {
            return Err(YrsEngineError::new(
                "CODEC_INVARIANT_FAILED",
                "Yrs XML attributes must be scalar Any values, not shared types",
            )
            .with_details(json!({
                "phase": "candidateMaterialization",
                "attribute": key,
            })));
        };
        if collect_lookup && lookup_attribute_work.failure().is_none() {
            lookup_attribute_work.observe(key, &value);
        }
        validate_any_projection(&value, &mut context.budget, 1)?;
        if let Some(expected) = projection.and_then(|projection| projection.attrs.get(key)) {
            local_match &= projection_value_matches_json(&value, expected);
            continue;
        }
        if removes_level && key == "level" {
            continue;
        }
        projected_attr_count = projected_attr_count.saturating_add(1);
        local_match &= any_matches_json(&value, expected_attrs.and_then(|attrs| attrs.get(key)));
    }
    local_match &= expected_attrs.map_or(0, Map::len) == projected_attr_count;

    let (is_void, is_textblock) = spec.map_or((true, false), |spec| {
        (spec.is_void, matches!(spec.role, NodeRole::TextBlock))
    });
    let observe_children = lookup.as_deref_mut().is_none_or(|lookup| {
        lookup.begin_element(
            AsRef::<yrs::branch::Branch>::as_ref(element).id(),
            lookup_attribute_work,
            is_void,
            is_textblock,
        )
    });
    let mut children = JsonMatchCursor::new(expected_content);
    for child in element.children(txn) {
        match_xml_out_json(
            child,
            txn,
            depth + 1,
            &mut children,
            &mut local_match,
            context,
            observe_children.then_some(lookup.as_deref_mut()).flatten(),
        )?;
    }
    if observe_children {
        if let Some(lookup) = lookup {
            lookup.end_container();
        }
    }
    let child_count = children.finish(&mut local_match);
    let has_attrs = projected_attr_count != 0;
    let has_content = child_count != 0;
    local_match &= expected_object.is_some_and(|object| {
        object.len() == 1 + usize::from(has_attrs) + usize::from(has_content)
            && object.contains_key("type")
            && object.contains_key("attrs") == has_attrs
            && object.contains_key("content") == has_content
    });
    *matched &= local_match;
    Ok(())
}

fn match_xml_text_json<T: ReadTxn>(
    text: &XmlTextRef,
    txn: &T,
    depth: usize,
    cursor: &mut JsonMatchCursor<'_>,
    matched: &mut bool,
    context: &mut JsonProjectionContext<'_, '_>,
    mut lookup: Option<&mut ImportLookupMaterializationCollector>,
) -> YrsEngineResult<()> {
    if lookup.as_deref().is_some_and(|lookup| lookup.has_failed()) {
        lookup = None;
    }
    let collect_lookup = lookup.is_some();
    let mut lookup_capture_work = ImportTextCaptureWork::new();
    for diff in text.diff(txn, YChange::identity) {
        context.budget.admit_raw_text_run(depth)?;
        let yrs::types::text::Diff {
            insert, attributes, ..
        } = diff;
        let yrs::Out::Any(any) = insert else {
            return Err(YrsEngineError::new(
                "CODEC_INVARIANT_FAILED",
                "Yrs XML text runs must contain scalar string values",
            )
            .with_details(json!({
                "phase": "candidateMaterialization",
                "field": "xmlTextRun"
            })));
        };
        let Any::String(text_value) = any else {
            return Err(YrsEngineError::new(
                "CODEC_INVARIANT_FAILED",
                "Yrs XML text runs must contain string values",
            )
            .with_details(json!({
                "phase": "candidateMaterialization",
                "field": "xmlTextRun"
            })));
        };
        if collect_lookup && lookup_capture_work.failure().is_none() {
            lookup_capture_work.observe(&text_value, attributes.as_deref());
        }
        context
            .budget
            .charge_computed_output(|| json_string_len(&text_value))?;
        if text_value.is_empty() {
            continue;
        }
        if let Some(attrs) = attributes.as_deref() {
            for value in attrs.values() {
                validate_any_projection(value, &mut context.budget, 1)?;
            }
        }
        cursor.observe_text(
            &text_value,
            attributes,
            context.schema,
            depth,
            &mut context.budget,
            matched,
        )?;
    }
    if let Some(lookup) = lookup {
        lookup.observe_text(
            AsRef::<yrs::branch::Branch>::as_ref(text).id(),
            lookup_capture_work,
        );
    }
    Ok(())
}
