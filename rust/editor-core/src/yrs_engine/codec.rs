use std::sync::Arc;

use serde_json::{json, Map, Value};
use yrs::any::Any;
use yrs::types::text::{Text, YChange};
use yrs::types::xml::{
    Xml, XmlElementPrelim, XmlElementRef, XmlFragment, XmlFragmentRef, XmlOut, XmlTextPrelim,
    XmlTextRef,
};
use yrs::types::{Attrs, ToJson};
use yrs::{ReadTxn, TransactionMut};

use crate::boundary::ResourceLimits;
use crate::schema::Schema;

use super::{YrsEngineError, YrsEngineResult};

pub(crate) struct YrsDocumentCodec<'a> {
    schema: &'a Schema,
    limits: &'a ResourceLimits,
}

impl<'a> YrsDocumentCodec<'a> {
    pub(crate) fn new(schema: &'a Schema, limits: &'a ResourceLimits) -> Self {
        Self { schema, limits }
    }

    pub(crate) fn read_json<T: ReadTxn>(
        &self,
        fragment: &XmlFragmentRef,
        txn: &T,
    ) -> YrsEngineResult<serde_json::Value> {
        let mut budget = ConversionBudget::new(self.limits);
        budget.admit_node(1)?;
        let mut content = Vec::new();
        for child in fragment.children(txn) {
            append_xml_out_json(child, txn, self.schema, 2, &mut budget, &mut content)?;
        }
        Ok(json!({
            "type": self.schema.doc_node_type(),
            "content": content,
        }))
    }

    pub(crate) fn apply_json<P: XmlFragment>(
        &self,
        parent: &P,
        txn: &mut TransactionMut<'_>,
        current: &serde_json::Value,
        next: &serde_json::Value,
    ) -> YrsEngineResult<()> {
        let mut budget = ConversionBudget::new(self.limits);
        budget.admit_node(1)?;
        let current_children = current
            .get("content")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let next_children = next
            .get("content")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        apply_children(parent, txn, current_children, next_children, 2, &mut budget)
    }
}

#[derive(Debug)]
struct ConversionBudget<'a> {
    limits: &'a ResourceLimits,
    nodes: usize,
    any_work: usize,
    output_bytes: usize,
}

impl<'a> ConversionBudget<'a> {
    fn new(limits: &'a ResourceLimits) -> Self {
        Self {
            limits,
            nodes: 0,
            any_work: 0,
            output_bytes: 0,
        }
    }

    fn admit_node(&mut self, depth: usize) -> YrsEngineResult<()> {
        self.nodes = self.nodes.saturating_add(1);
        if self.nodes > self.limits.max_document_nodes {
            return Err(YrsEngineError::limit(
                "DOCUMENT_LIMIT_EXCEEDED",
                self.limits.max_document_nodes,
                self.nodes,
            ));
        }
        self.admit_traversal_depth(depth)
    }

    fn admit_traversal_depth(&self, depth: usize) -> YrsEngineResult<()> {
        if depth > self.limits.max_document_depth {
            return Err(YrsEngineError::limit(
                "DOCUMENT_LIMIT_EXCEEDED",
                self.limits.max_document_depth,
                depth,
            ));
        }
        Ok(())
    }

    fn admit_any(&mut self, depth: usize, amount: usize) -> YrsEngineResult<()> {
        if depth > self.limits.max_document_depth {
            return Err(YrsEngineError::limit(
                "DOCUMENT_LIMIT_EXCEEDED",
                self.limits.max_document_depth,
                depth,
            )
            .with_details(json!({
                "phase": "candidateMaterialization",
                "dimension": "anyDepth"
            })));
        }
        self.any_work = self.any_work.saturating_add(amount);
        let limit = self.limits.max_document_nodes.saturating_mul(128);
        if self.any_work > limit {
            return Err(
                YrsEngineError::limit("DOCUMENT_LIMIT_EXCEEDED", limit, self.any_work)
                    .with_details(json!({
                        "phase": "candidateMaterialization",
                        "dimension": "anyWork"
                    })),
            );
        }
        Ok(())
    }

    fn charge_output(&mut self, bytes: usize) -> YrsEngineResult<()> {
        self.output_bytes = self.output_bytes.saturating_add(bytes);
        if self.output_bytes > self.limits.max_input_bytes {
            return Err(YrsEngineError::limit(
                "DOCUMENT_LIMIT_EXCEEDED",
                self.limits.max_input_bytes,
                self.output_bytes,
            )
            .with_details(json!({
                "phase": "candidateMaterialization",
                "dimension": "outputBytes"
            })));
        }
        Ok(())
    }
}

fn append_xml_out_json<T: ReadTxn>(
    node: XmlOut,
    txn: &T,
    schema: &Schema,
    depth: usize,
    budget: &mut ConversionBudget<'_>,
    output: &mut Vec<Value>,
) -> YrsEngineResult<()> {
    match node {
        XmlOut::Element(element) => {
            output.push(xml_element_to_json(&element, txn, schema, depth, budget)?)
        }
        XmlOut::Text(text) => append_xml_text_json(&text, txn, schema, depth, budget, output)?,
        XmlOut::Fragment(fragment) => {
            budget.admit_traversal_depth(depth)?;
            for child in fragment.children(txn) {
                append_xml_out_json(child, txn, schema, depth + 1, budget, output)?;
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
) -> YrsEngineResult<Value> {
    budget.admit_node(depth)?;
    let mut object = Map::new();
    let mut attrs = Map::new();
    for (key, value) in element.attributes(txn) {
        attrs.insert(
            key.to_string(),
            any_to_json(&value.to_json(txn), budget, 1)?,
        );
    }
    let node_type = json_node_type_for_element(element.tag(), &mut attrs);
    object.insert("type".to_string(), Value::String(node_type));
    if !attrs.is_empty() {
        object.insert("attrs".to_string(), Value::Object(attrs));
    }

    let mut children = Vec::new();
    for child in element.children(txn) {
        append_xml_out_json(child, txn, schema, depth + 1, budget, &mut children)?;
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
) -> YrsEngineResult<()> {
    for diff in text.diff(txn, YChange::identity) {
        let yrs::Out::Any(any) = diff.insert else {
            continue;
        };
        let Value::String(text_value) = any_to_json(&any, budget, 1)? else {
            continue;
        };
        if text_value.is_empty() {
            continue;
        }
        budget.admit_node(depth)?;
        let mut object = Map::new();
        object.insert("type".to_string(), Value::String("text".to_string()));
        object.insert("text".to_string(), Value::String(text_value));
        let marks = attrs_to_marks(diff.attributes.as_deref(), schema, budget)?;
        if !marks.is_empty() {
            object.insert("marks".to_string(), Value::Array(marks));
        }
        output.push(Value::Object(object));
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

fn apply_children<P: XmlFragment>(
    parent: &P,
    txn: &mut TransactionMut<'_>,
    old_children: &[Value],
    new_children: &[Value],
    depth: usize,
    budget: &mut ConversionBudget<'_>,
) -> YrsEngineResult<()> {
    let mut prefix = 0usize;
    while prefix < old_children.len()
        && prefix < new_children.len()
        && nodes_are_compatible(&old_children[prefix], &new_children[prefix])
    {
        apply_child_at(
            parent,
            txn,
            prefix as u32,
            &old_children[prefix],
            &new_children[prefix],
            depth,
            budget,
        )?;
        prefix += 1;
    }

    let mut old_suffix = old_children.len();
    let mut new_suffix = new_children.len();
    while old_suffix > prefix
        && new_suffix > prefix
        && nodes_are_compatible(&old_children[old_suffix - 1], &new_children[new_suffix - 1])
    {
        old_suffix -= 1;
        new_suffix -= 1;
    }

    let old_mid_len = old_suffix.saturating_sub(prefix) as u32;
    if old_mid_len > 0 {
        parent.remove_range(txn, prefix as u32, old_mid_len);
    }

    for (offset, node) in new_children[prefix..new_suffix].iter().enumerate() {
        insert_json_node(
            parent,
            txn,
            prefix as u32 + offset as u32,
            node,
            depth,
            budget,
        )?;
    }

    let suffix_len = old_children.len().saturating_sub(old_suffix);
    for offset in 0..suffix_len {
        let new_index = new_suffix + offset;
        let parent_index = (new_suffix + offset) as u32;
        apply_child_at(
            parent,
            txn,
            parent_index,
            &old_children[old_suffix + offset],
            &new_children[new_index],
            depth,
            budget,
        )?;
    }
    Ok(())
}

fn apply_child_at<P: XmlFragment>(
    parent: &P,
    txn: &mut TransactionMut<'_>,
    index: u32,
    old_node: &Value,
    new_node: &Value,
    depth: usize,
    budget: &mut ConversionBudget<'_>,
) -> YrsEngineResult<()> {
    budget.admit_node(depth)?;
    let Some(current) = parent.get(txn, index) else {
        return Ok(());
    };

    match current {
        XmlOut::Element(element) => {
            apply_element_node(&element, txn, old_node, new_node, depth, budget)
        }
        XmlOut::Text(text) => {
            apply_text_node(&text, txn, old_node, new_node);
            Ok(())
        }
        XmlOut::Fragment(_) => Ok(()),
    }
}

fn apply_element_node(
    element: &XmlElementRef,
    txn: &mut TransactionMut<'_>,
    old_node: &Value,
    new_node: &Value,
    depth: usize,
    budget: &mut ConversionBudget<'_>,
) -> YrsEngineResult<()> {
    let old_attrs = old_node.get("attrs").and_then(Value::as_object);
    let new_attrs = new_node.get("attrs").and_then(Value::as_object);

    if old_attrs != new_attrs {
        let existing = element
            .attributes(txn)
            .map(|(key, _)| key.to_string())
            .collect::<Vec<_>>();
        for key in existing {
            element.remove_attribute(txn, &key);
        }
        if let Some(attrs) = new_attrs {
            for (key, value) in attrs {
                element.insert_attribute(txn, key.as_str(), json_to_any(value));
            }
        }
    }

    let old_children = old_node
        .get("content")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let new_children = new_node
        .get("content")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    apply_children(element, txn, old_children, new_children, depth + 1, budget)
}

fn apply_text_node(
    text: &XmlTextRef,
    txn: &mut TransactionMut<'_>,
    old_node: &Value,
    new_node: &Value,
) {
    let old_marks = normalize_marks(old_node.get("marks").and_then(Value::as_array));
    let new_marks = normalize_marks(new_node.get("marks").and_then(Value::as_array));
    let old_text = old_node.get("text").and_then(Value::as_str).unwrap_or("");
    let new_text = new_node.get("text").and_then(Value::as_str).unwrap_or("");

    if old_marks != new_marks {
        let len = text.len(txn);
        if len > 0 {
            text.remove_range(txn, 0, len);
        }
        if !new_text.is_empty() {
            text.insert_with_attributes(txn, 0, new_text, marks_to_attrs(Some(&new_marks)));
        }
        return;
    }

    let (prefix, old_suffix, new_suffix) = shared_text_bounds(old_text, new_text);
    let prefix_utf16 = old_text[..prefix].encode_utf16().count() as u32;
    let remove_len = old_text[prefix..old_suffix].encode_utf16().count() as u32;
    if remove_len > 0 {
        text.remove_range(txn, prefix_utf16, remove_len);
    }

    let insert_text = &new_text[prefix..new_suffix];
    if !insert_text.is_empty() {
        text.insert_with_attributes(
            txn,
            prefix_utf16,
            insert_text,
            marks_to_attrs(Some(&new_marks)),
        );
    }
}

fn shared_text_bounds(old_text: &str, new_text: &str) -> (usize, usize, usize) {
    let mut prefix = 0usize;
    let mut old_iter = old_text.char_indices().peekable();
    let mut new_iter = new_text.char_indices().peekable();

    while let (Some((old_index, old_char)), Some((new_index, new_char))) =
        (old_iter.peek().copied(), new_iter.peek().copied())
    {
        if old_char != new_char || old_index != prefix || new_index != prefix {
            break;
        }
        prefix += old_char.len_utf8();
        old_iter.next();
        new_iter.next();
    }

    let mut old_suffix = old_text.len();
    let mut new_suffix = new_text.len();
    let old_tail = old_text[prefix..].chars().rev().collect::<Vec<_>>();
    let new_tail = new_text[prefix..].chars().rev().collect::<Vec<_>>();
    for (old_char, new_char) in old_tail.iter().zip(new_tail.iter()) {
        if old_char != new_char {
            break;
        }
        old_suffix -= old_char.len_utf8();
        new_suffix -= new_char.len_utf8();
    }

    (prefix, old_suffix, new_suffix)
}

fn insert_json_node<P: XmlFragment>(
    parent: &P,
    txn: &mut TransactionMut<'_>,
    index: u32,
    node: &Value,
    depth: usize,
    budget: &mut ConversionBudget<'_>,
) -> YrsEngineResult<()> {
    budget.admit_node(depth)?;
    let node_type = node.get("type").and_then(Value::as_str).unwrap_or("");
    if node_type == "text" {
        let text_value = node.get("text").and_then(Value::as_str).unwrap_or("");
        let text = parent.insert(txn, index, XmlTextPrelim::new(""));
        if !text_value.is_empty() {
            text.insert_with_attributes(
                txn,
                0,
                text_value,
                marks_to_attrs(node.get("marks").and_then(Value::as_array)),
            );
        }
        return Ok(());
    }

    let mut attrs = node
        .get("attrs")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let element_name = element_name_for_json_node(node_type, &mut attrs);
    let element = parent.insert(txn, index, XmlElementPrelim::empty(element_name.as_str()));
    for (key, value) in &attrs {
        element.insert_attribute(txn, key.as_str(), json_to_any(value));
    }
    if let Some(children) = node.get("content").and_then(Value::as_array) {
        for child in children {
            let next_index = element.len(txn);
            insert_json_node(&element, txn, next_index, child, depth + 1, budget)?;
        }
    }
    Ok(())
}

fn json_node_type_for_element(tag: &str, attrs: &mut Map<String, Value>) -> String {
    if tag == "heading" {
        if let Some(level) = parse_heading_level_value(attrs.get("level")) {
            attrs.remove("level");
            return format!("h{level}");
        }
    }

    tag.to_string()
}

fn element_name_for_json_node(node_type: &str, attrs: &mut Map<String, Value>) -> String {
    if let Some(level) = heading_level_from_internal_node_type(node_type) {
        attrs.insert("level".to_string(), Value::Number(u64::from(level).into()));
        return "heading".to_string();
    }

    node_type.to_string()
}

fn heading_level_from_internal_node_type(node_type: &str) -> Option<u8> {
    let suffix = node_type.strip_prefix('h')?;
    if suffix.len() != 1 {
        return None;
    }
    let level = suffix.parse::<u8>().ok()?;
    (1..=6).contains(&level).then_some(level)
}

fn parse_heading_level_value(value: Option<&Value>) -> Option<u8> {
    let value = value?;
    let level = match value {
        Value::Number(number) => {
            if let Some(value) = number.as_u64() {
                u8::try_from(value).ok()?
            } else if let Some(value) = number.as_i64() {
                u8::try_from(value).ok()?
            } else if let Some(value) = number.as_f64() {
                if !value.is_finite() || value.fract() != 0.0 {
                    return None;
                }
                u8::try_from(value as i64).ok()?
            } else {
                return None;
            }
        }
        Value::String(value) => value.parse::<u8>().ok()?,
        _ => return None,
    };

    (1..=6).contains(&level).then_some(level)
}

fn normalize_marks(marks: Option<&Vec<Value>>) -> Vec<Value> {
    let mut normalized = marks.cloned().unwrap_or_default();
    normalized.sort_by(|left, right| {
        let left_name = left.get("type").and_then(Value::as_str).unwrap_or("");
        let right_name = right.get("type").and_then(Value::as_str).unwrap_or("");
        left_name.cmp(right_name)
    });
    normalized
}

fn marks_to_attrs(marks: Option<&Vec<Value>>) -> Attrs {
    let mut attrs = Attrs::default();
    let Some(marks) = marks else {
        return attrs;
    };
    for mark in marks {
        let Some(mark_type) = mark.get("type").and_then(Value::as_str) else {
            continue;
        };
        let value = mark
            .get("attrs")
            .map(json_to_any)
            .unwrap_or_else(|| Any::Bool(true));
        attrs.insert(mark_type.into(), value);
    }
    attrs
}

fn nodes_are_compatible(old_node: &Value, new_node: &Value) -> bool {
    let old_type = old_node.get("type").and_then(Value::as_str);
    let new_type = new_node.get("type").and_then(Value::as_str);
    if old_type != new_type {
        return false;
    }

    match old_type {
        Some("text") => {
            normalize_marks(old_node.get("marks").and_then(Value::as_array))
                == normalize_marks(new_node.get("marks").and_then(Value::as_array))
        }
        Some(_) => true,
        None => false,
    }
}

fn json_to_any(value: &Value) -> Any {
    match value {
        Value::Null => Any::Null,
        Value::Bool(value) => Any::Bool(*value),
        Value::Number(number) => {
            if let Some(value) = number.as_i64() {
                Any::BigInt(value)
            } else if let Some(value) = number.as_u64() {
                Any::Number(value as f64)
            } else if let Some(value) = number.as_f64() {
                Any::Number(value)
            } else {
                Any::Null
            }
        }
        Value::String(value) => Any::String(value.clone().into()),
        Value::Array(values) => Any::Array(values.iter().map(json_to_any).collect()),
        Value::Object(values) => Any::Map(Arc::new(
            values
                .iter()
                .map(|(key, value)| (key.clone(), json_to_any(value)))
                .collect(),
        )),
    }
}

#[cfg(test)]
fn any_to_json_bounded(value: &Any, limits: &ResourceLimits) -> YrsEngineResult<Value> {
    let mut budget = ConversionBudget::new(limits);
    any_to_json(value, &mut budget, 1)
}

fn any_to_json(
    value: &Any,
    budget: &mut ConversionBudget<'_>,
    depth: usize,
) -> YrsEngineResult<Value> {
    budget.admit_any(depth, 1)?;
    match value {
        Any::Null | Any::Undefined => {
            budget.charge_output(4)?;
            Ok(Value::Null)
        }
        Any::Bool(value) => {
            budget.charge_output(if *value { 4 } else { 5 })?;
            Ok(Value::Bool(*value))
        }
        Any::Number(value) => {
            let number = serde_json::Number::from_f64(*value);
            let bytes = number.as_ref().map_or(4, |number| number.to_string().len());
            budget.charge_output(bytes)?;
            Ok(number.map(Value::Number).unwrap_or(Value::Null))
        }
        Any::BigInt(value) => {
            budget.charge_output(value.to_string().len())?;
            Ok(Value::Number((*value).into()))
        }
        Any::String(value) => {
            budget.charge_output(json_string_len(value))?;
            Ok(Value::String(value.to_string()))
        }
        Any::Buffer(value) => {
            budget.admit_any(depth, value.len())?;
            budget.charge_output(2usize.saturating_add(value.len().saturating_sub(1)))?;
            let mut output = Vec::new();
            for byte in value.iter() {
                budget.charge_output(decimal_u8_len(*byte))?;
                output.push(Value::Number((*byte).into()));
            }
            Ok(Value::Array(output))
        }
        Any::Array(values) => {
            budget.charge_output(2usize.saturating_add(values.len().saturating_sub(1)))?;
            let mut output = Vec::new();
            for value in values.iter() {
                output.push(any_to_json(value, budget, depth + 1)?);
            }
            Ok(Value::Array(output))
        }
        Any::Map(values) => {
            budget.charge_output(2usize.saturating_add(values.len().saturating_sub(1)))?;
            let mut output = Map::new();
            for (key, value) in values.iter() {
                budget.charge_output(json_string_len(key).saturating_add(1))?;
                output.insert(key.to_string(), any_to_json(value, budget, depth + 1)?);
            }
            Ok(Value::Object(output))
        }
    }
}

fn json_string_len(value: &str) -> usize {
    value.chars().fold(2usize, |bytes, character| {
        bytes.saturating_add(match character {
            '"' | '\\' | '\u{0008}' | '\u{000c}' | '\n' | '\r' | '\t' => 2,
            character if character <= '\u{001f}' => 6,
            character => character.len_utf8(),
        })
    })
}

fn decimal_u8_len(value: u8) -> usize {
    if value >= 100 {
        3
    } else if value >= 10 {
        2
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::{any_to_json_bounded, attrs_to_marks, marks_to_attrs, YrsDocumentCodec};
    use crate::boundary::ResourceLimits;
    use crate::schema::presets::tiptap_schema;
    use serde_json::{json, Value};
    use yrs::{Doc, OffsetKind, Options, ReadTxn, Transact, WriteTxn};

    fn utf16_doc() -> Doc {
        let options = Options {
            offset_kind: OffsetKind::Utf16,
            ..Default::default()
        };
        Doc::with_options(options)
    }

    fn empty_json(document_root_type: &str) -> Value {
        json!({
            "type": document_root_type,
            "content": [],
        })
    }

    fn round_trip(next: Value) -> Value {
        let schema = tiptap_schema();
        let limits = ResourceLimits::default();
        let codec = YrsDocumentCodec::new(&schema, &limits);
        let doc = utf16_doc();
        {
            let mut txn = doc.transact_mut();
            let fragment = txn.get_or_insert_xml_fragment("prosemirror");
            codec
                .apply_json(&fragment, &mut txn, &empty_json("doc"), &next)
                .unwrap();
        }
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        codec.read_json(&fragment, &txn).unwrap()
    }

    #[test]
    fn round_trips_heading_mark_attrs_emoji_and_combining_text() {
        let next = json!({
            "type": "doc",
            "content": [{
                "type": "h2",
                "content": [{
                    "type": "text",
                    "text": "A😀e\u{301}",
                    "marks": [{
                        "type": "link",
                        "attrs": {
                            "href": "https://example.test",
                            "target": "_blank"
                        }
                    }]
                }]
            }]
        });

        assert_eq!(round_trip(next.clone()), next);
    }

    #[test]
    fn borrowed_mark_conversion_preserves_exact_sorted_attrs() {
        let marks = vec![
            json!({ "type": "italic" }),
            json!({
                "type": "link",
                "attrs": {
                    "href": "https://example.test/😀",
                    "title": "e\u{301} אב"
                }
            }),
            json!({ "type": "bold" }),
        ];

        let attrs = marks_to_attrs(Some(&marks));
        let schema = tiptap_schema();
        let limits = ResourceLimits::default();
        let mut budget = super::ConversionBudget::new(&limits);
        assert_eq!(
            attrs_to_marks(Some(&attrs), &schema, &mut budget).unwrap(),
            vec![
                json!({ "type": "bold" }),
                json!({ "type": "italic" }),
                json!({
                    "type": "link",
                    "attrs": {
                        "href": "https://example.test/😀",
                        "title": "e\u{301} אב"
                    }
                }),
            ]
        );
    }

    #[test]
    fn shared_codec_preserves_multimark_unicode_and_opaque_payload_exactly() {
        let input = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{
                    "type": "text",
                    "text": "A😀e\u{301} אב",
                    "marks": [
                        { "type": "italic" },
                        { "type": "link", "attrs": { "href": "https://example.test/😀" } },
                        { "type": "bold" }
                    ]
                }]
            }, {
                "type": "__opaque_json",
                "attrs": {
                    "original_type": "callout",
                    "opaque_placement": "block",
                    "original_json": {
                        "type": "callout",
                        "attrs": { "payload": ["😀", "e\u{301}", "אב", null] }
                    }
                }
            }]
        });
        let expected = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{
                    "type": "text",
                    "text": "A😀e\u{301} אב",
                    "marks": [
                        { "type": "bold" },
                        { "type": "italic" },
                        { "type": "link", "attrs": { "href": "https://example.test/😀" } }
                    ]
                }]
            }, {
                "type": "__opaque_json",
                "attrs": {
                    "original_type": "callout",
                    "opaque_placement": "block",
                    "original_json": {
                        "type": "callout",
                        "attrs": { "payload": ["😀", "e\u{301}", "אב", null] }
                    }
                }
            }]
        });

        assert_eq!(round_trip(input), expected);
    }

    #[test]
    fn round_trips_list_attrs_and_inline_and_block_void_nodes() {
        let next = json!({
            "type": "doc",
            "content": [
                {
                    "type": "orderedList",
                    "attrs": { "start": 3 },
                    "content": [{
                        "type": "listItem",
                        "content": [{
                            "type": "paragraph",
                            "content": [
                                { "type": "text", "text": "before" },
                                { "type": "hardBreak" },
                                { "type": "text", "text": "after" }
                            ]
                        }]
                    }]
                },
                { "type": "horizontalRule" },
                {
                    "type": "image",
                    "attrs": {
                        "src": "https://example.test/image.png",
                        "alt": "example"
                    }
                }
            ]
        });

        assert_eq!(round_trip(next.clone()), next);
    }

    #[test]
    fn round_trips_opaque_json_nodes_without_changing_payloads() {
        let next = json!({
            "type": "doc",
            "content": [{
                "type": "__opaque_json",
                "attrs": {
                    "original_type": "callout",
                    "opaque_placement": "block",
                    "original_json": {
                        "type": "callout",
                        "attrs": {
                            "kind": "warning",
                            "metadata": [true, null, { "rank": 2 }]
                        },
                        "content": [{ "type": "text", "text": "preserve me" }]
                    }
                }
            }]
        });

        assert_eq!(round_trip(next.clone()), next);
    }

    #[test]
    fn read_and_write_share_exact_node_limit_accounting() {
        let schema = tiptap_schema();
        let next = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{ "type": "text", "text": "three nodes" }]
            }]
        });
        let exact_limits = ResourceLimits {
            max_document_nodes: 3,
            ..Default::default()
        };
        let exact_codec = YrsDocumentCodec::new(&schema, &exact_limits);
        let doc = utf16_doc();
        {
            let mut txn = doc.transact_mut();
            let fragment = txn.get_or_insert_xml_fragment("prosemirror");
            exact_codec
                .apply_json(&fragment, &mut txn, &empty_json("doc"), &next)
                .unwrap();
        }
        {
            let txn = doc.transact();
            let fragment = txn.get_xml_fragment("prosemirror").unwrap();
            assert_eq!(exact_codec.read_json(&fragment, &txn).unwrap(), next);
        }

        let mut rejected_limits = exact_limits.clone();
        rejected_limits.max_document_nodes = 2;
        let rejected_codec = YrsDocumentCodec::new(&schema, &rejected_limits);
        let write_doc = utf16_doc();
        let write_error = {
            let mut txn = write_doc.transact_mut();
            let fragment = txn.get_or_insert_xml_fragment("prosemirror");
            rejected_codec
                .apply_json(&fragment, &mut txn, &empty_json("doc"), &next)
                .unwrap_err()
        };
        let read_error = {
            let txn = doc.transact();
            let fragment = txn.get_xml_fragment("prosemirror").unwrap();
            rejected_codec.read_json(&fragment, &txn).unwrap_err()
        };

        for error in [write_error, read_error] {
            assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
            assert_eq!(error.limit, Some(2));
            assert_eq!(error.actual, Some(3));
        }
    }

    #[test]
    fn read_and_write_share_exact_depth_limit_accounting() {
        let schema = tiptap_schema();
        let next = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{ "type": "text", "text": "depth three" }]
            }]
        });
        let exact_limits = ResourceLimits {
            max_document_depth: 3,
            ..Default::default()
        };
        let exact_codec = YrsDocumentCodec::new(&schema, &exact_limits);
        let doc = utf16_doc();
        {
            let mut txn = doc.transact_mut();
            let fragment = txn.get_or_insert_xml_fragment("prosemirror");
            exact_codec
                .apply_json(&fragment, &mut txn, &empty_json("doc"), &next)
                .unwrap();
        }
        {
            let txn = doc.transact();
            let fragment = txn.get_xml_fragment("prosemirror").unwrap();
            assert_eq!(exact_codec.read_json(&fragment, &txn).unwrap(), next);
        }

        let mut rejected_limits = exact_limits.clone();
        rejected_limits.max_document_depth = 2;
        let rejected_codec = YrsDocumentCodec::new(&schema, &rejected_limits);
        let write_doc = utf16_doc();
        let write_error = {
            let mut txn = write_doc.transact_mut();
            let fragment = txn.get_or_insert_xml_fragment("prosemirror");
            rejected_codec
                .apply_json(&fragment, &mut txn, &empty_json("doc"), &next)
                .unwrap_err()
        };
        let read_error = {
            let txn = doc.transact();
            let fragment = txn.get_xml_fragment("prosemirror").unwrap();
            rejected_codec.read_json(&fragment, &txn).unwrap_err()
        };

        for error in [write_error, read_error] {
            assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
            assert_eq!(error.limit, Some(2));
            assert_eq!(error.actual, Some(3));
        }
    }

    #[test]
    fn any_materialization_has_exact_depth_work_and_output_boundaries() {
        let nested = yrs::Any::Array(vec![yrs::Any::Array(vec![yrs::Any::Null].into())].into());
        let exact_depth = ResourceLimits {
            max_document_depth: 3,
            ..ResourceLimits::default()
        };
        assert_eq!(
            any_to_json_bounded(&nested, &exact_depth).unwrap(),
            serde_json::json!([[null]])
        );
        let depth_error = any_to_json_bounded(
            &nested,
            &ResourceLimits {
                max_document_depth: 2,
                ..ResourceLimits::default()
            },
        )
        .unwrap_err();
        assert_eq!(depth_error.code, "DOCUMENT_LIMIT_EXCEEDED");
        assert_eq!(depth_error.limit, Some(2));
        assert_eq!(depth_error.actual, Some(3));

        let exact_output = ResourceLimits {
            max_input_bytes: 3,
            ..ResourceLimits::default()
        };
        assert_eq!(
            any_to_json_bounded(&yrs::Any::String("x".into()), &exact_output).unwrap(),
            serde_json::json!("x")
        );
        let output_error = any_to_json_bounded(
            &yrs::Any::String("x".into()),
            &ResourceLimits {
                max_input_bytes: 2,
                ..ResourceLimits::default()
            },
        )
        .unwrap_err();
        assert_eq!(output_error.code, "DOCUMENT_LIMIT_EXCEEDED");
        assert_eq!(output_error.limit, Some(2));
        assert_eq!(output_error.actual, Some(3));

        let exact_items = yrs::Any::Array(vec![yrs::Any::Null; 127].into());
        assert!(any_to_json_bounded(
            &exact_items,
            &ResourceLimits {
                max_document_nodes: 1,
                ..ResourceLimits::default()
            }
        )
        .is_ok());
        let over_items = yrs::Any::Array(vec![yrs::Any::Null; 128].into());
        let work_error = any_to_json_bounded(
            &over_items,
            &ResourceLimits {
                max_document_nodes: 1,
                ..ResourceLimits::default()
            },
        )
        .unwrap_err();
        assert_eq!(work_error.code, "DOCUMENT_LIMIT_EXCEEDED");
        assert_eq!(work_error.limit, Some(128));
        assert_eq!(work_error.actual, Some(129));
    }
}
