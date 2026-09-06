fn append_xml_out_json<T: ReadTxn>(
    node: XmlOut,
    txn: &T,
    schema: &Schema,
    depth: usize,
    budget: &mut ConversionBudget<'_>,
    output: &mut Vec<Value>,
    lookup: Option<&mut ImportLookupMaterializationCollector>,
) -> YrsEngineResult<()> {
    stacker::maybe_grow(
        RECURSION_RED_ZONE_BYTES,
        RECURSION_STACK_SEGMENT_BYTES,
        || append_xml_out_json_inner(node, txn, schema, depth, budget, output, lookup),
    )
}

fn append_xml_out_json_inner<T: ReadTxn>(
    node: XmlOut,
    txn: &T,
    schema: &Schema,
    depth: usize,
    budget: &mut ConversionBudget<'_>,
    output: &mut Vec<Value>,
    mut lookup: Option<&mut ImportLookupMaterializationCollector>,
) -> YrsEngineResult<()> {
    if lookup.as_deref().is_some_and(|lookup| lookup.has_failed()) {
        lookup = None;
    }
    match node {
        XmlOut::Element(element) => output.push(xml_element_to_json(
            &element, txn, schema, depth, budget, lookup,
        )?),
        XmlOut::Text(text) => {
            append_xml_text_json(&text, txn, schema, depth, budget, output, lookup)?
        }
        XmlOut::Fragment(fragment) => {
            budget.admit_traversal_depth(depth)?;
            let mut lookup = lookup;
            if let Some(lookup) = lookup.as_deref_mut() {
                lookup.begin_fragment();
            }
            for child in fragment.children(txn) {
                append_xml_out_json(
                    child,
                    txn,
                    schema,
                    depth + 1,
                    budget,
                    output,
                    lookup.as_deref_mut(),
                )?;
            }
            if let Some(lookup) = lookup {
                lookup.end_container();
            }
        }
    }
    Ok(())
}

fn xml_element_to_json<T: ReadTxn>(
    element: &XmlElementRef,
    txn: &T,
    schema: &Schema,
    depth: usize,
    budget: &mut ConversionBudget<'_>,
    mut lookup: Option<&mut ImportLookupMaterializationCollector>,
) -> YrsEngineResult<Value> {
    if lookup.as_deref().is_some_and(|lookup| lookup.has_failed()) {
        lookup = None;
    }
    let collect_lookup = lookup.is_some();
    budget.admit_node(depth)?;
    let mut object = Map::new();
    let mut attrs = Map::new();
    let mut lookup_attribute_work = ImportElementAttributeWork::new();
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
        attrs.insert(key.to_string(), any_to_json(&value, budget, 1)?);
    }
    let raw_tag = element.tag();
    let normalized_node_type = normalized_wire_element_node_type(element, txn);
    let spec = wire_element_node_spec(element, txn, schema);
    let node_type = spec.map_or(normalized_node_type.as_str(), |spec| spec.name.as_str());
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
    if normalized_node_type != raw_tag.as_ref()
        && node_type != raw_tag.as_ref()
        && spec.is_none_or(|spec| !spec.attrs.contains_key("level"))
    {
        attrs.remove("level");
    }
    let projected_type = spec
        .and_then(|spec| spec.json_projection.as_ref())
        .map_or(node_type, |projection| projection.node_type.as_str());
    if let Some(projection) = spec.and_then(|spec| spec.json_projection.as_ref()) {
        attrs.extend(projection.attrs.iter().map(|(name, value)| {
            (
                name.clone(),
                crate::boundary::clone_json_value_stack_safe(value),
            )
        }));
    }
    object.insert(
        "type".to_string(),
        Value::String(projected_type.to_string()),
    );
    if !attrs.is_empty() {
        object.insert("attrs".to_string(), Value::Object(attrs));
    }

    let mut children = Vec::new();
    for child in element.children(txn) {
        append_xml_out_json(
            child,
            txn,
            schema,
            depth + 1,
            budget,
            &mut children,
            observe_children.then_some(lookup.as_deref_mut()).flatten(),
        )?;
    }
    if observe_children {
        if let Some(lookup) = lookup {
            lookup.end_container();
        }
    }
    if !children.is_empty() {
        object.insert("content".to_string(), Value::Array(children));
    }

    Ok(Value::Object(object))
}

fn append_xml_text_json<T: ReadTxn>(
    text: &XmlTextRef,
    txn: &T,
    schema: &Schema,
    depth: usize,
    budget: &mut ConversionBudget<'_>,
    output: &mut Vec<Value>,
    mut lookup: Option<&mut ImportLookupMaterializationCollector>,
) -> YrsEngineResult<()> {
    if lookup.as_deref().is_some_and(|lookup| lookup.has_failed()) {
        lookup = None;
    }
    let collect_lookup = lookup.is_some();
    let mut lookup_capture_work = ImportTextCaptureWork::new();
    for diff in text.diff(txn, YChange::identity) {
        budget.admit_raw_text_run(depth)?;
        let yrs::Out::Any(any) = diff.insert else {
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
            lookup_capture_work.observe(&text_value, diff.attributes.as_deref());
        }
        budget.charge_computed_output(|| json_string_len(&text_value))?;
        let text_value = text_value.to_string();
        if text_value.is_empty() {
            continue;
        }
        let marks = attrs_to_marks(diff.attributes.as_deref(), schema, budget)?;
        let marks = (!marks.is_empty()).then_some(Value::Array(marks));
        if let Some(Value::Object(previous)) = output.last_mut() {
            let compatible = previous.get("type").and_then(Value::as_str) == Some("text")
                && previous.get("marks") == marks.as_ref();
            if compatible {
                if let Some(Value::String(previous_text)) = previous.get_mut("text") {
                    previous_text.push_str(&text_value);
                    continue;
                }
            }
        }

        budget.admit_node(depth)?;
        let mut object = Map::new();
        object.insert("type".to_string(), Value::String("text".to_string()));
        object.insert("text".to_string(), Value::String(text_value));
        if let Some(marks) = marks {
            object.insert("marks".to_string(), marks);
        }
        output.push(Value::Object(object));
    }
    if let Some(lookup) = lookup {
        lookup.observe_text(
            AsRef::<yrs::branch::Branch>::as_ref(text).id(),
            lookup_capture_work,
        );
    }
    Ok(())
}

fn attrs_to_marks(
    attrs: Option<&Attrs>,
    schema: &Schema,
    budget: &mut ConversionBudget<'_>,
) -> YrsEngineResult<Vec<Value>> {
    let mut marks = Vec::new();
    let Some(attrs) = attrs else {
        return Ok(marks);
    };
    for (name, value) in attrs {
        let mut object = Map::new();
        object.insert("type".to_string(), Value::String(name.to_string()));
        match any_to_json(value, budget, 1)? {
            Value::Bool(true) | Value::Null => {}
            other => {
                object.insert("attrs".to_string(), other);
            }
        }
        marks.push(Value::Object(object));
    }
    marks.sort_by(|left, right| {
        let left_name = left.get("type").and_then(Value::as_str).unwrap_or("");
        let right_name = right.get("type").and_then(Value::as_str).unwrap_or("");
        schema
            .mark_rank(left_name)
            .unwrap_or(usize::MAX)
            .cmp(&schema.mark_rank(right_name).unwrap_or(usize::MAX))
            .then_with(|| left_name.cmp(right_name))
    });
    Ok(marks)
}
