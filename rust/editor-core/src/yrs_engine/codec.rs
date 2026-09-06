use std::sync::Arc;

use serde_json::{json, Map, Value};
use yrs::any::Any;
use yrs::types::text::{Text, YChange};
use yrs::types::xml::{
    Xml, XmlElementPrelim, XmlElementRef, XmlFragment, XmlFragmentRef, XmlOut, XmlTextPrelim,
    XmlTextRef,
};
use yrs::types::Attrs;
use yrs::{ReadTxn, TransactionMut};

use crate::boundary::ResourceLimits;
use crate::schema::{json_projection_values_equal, NodeJsonProjection, NodeRole, NodeSpec, Schema};

use super::mutation::{
    ImportElementAttributeWork, ImportLookupMaterializationCollector, ImportTextCaptureWork,
};
use super::{raw_storage_work_limit, YrsEngineError, YrsEngineResult};

const RECURSION_RED_ZONE_BYTES: usize = 64 * 1024;
const RECURSION_STACK_SEGMENT_BYTES: usize = 1024 * 1024;

#[cfg(test)]
std::thread_local! {
    static JSON_PROJECTION_MATERIALIZATION_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn take_json_projection_materialization_count_for_test() -> usize {
    JSON_PROJECTION_MATERIALIZATION_COUNT.with(|count| count.replace(0))
}

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
        self.read_json_with_budget(fragment, txn, ConversionBudget::new(self.limits), None)
    }

    pub(crate) fn matches_validated_json_with_lookup<T: ReadTxn>(
        &self,
        fragment: &XmlFragmentRef,
        txn: &T,
        expected: &Value,
    ) -> (
        YrsEngineResult<bool>,
        Option<super::mutation::ImportLookupMaterialization>,
    ) {
        let root_width = usize::try_from(fragment.len(txn)).unwrap_or(usize::MAX);
        let mut collector = ImportLookupMaterializationCollector::new(
            0,
            AsRef::<yrs::branch::Branch>::as_ref(fragment).id(),
            root_width,
            None,
        );
        let matched = (|| {
            let mut context = JsonProjectionContext {
                schema: self.schema,
                budget: ConversionBudget::for_validated_source(self.limits),
            };
            context.budget.admit_node(1)?;
            let expected_object = expected.as_object();
            let expected_content = expected_object
                .and_then(|object| object.get("content"))
                .and_then(Value::as_array)
                .map(Vec::as_slice);
            let mut matched = expected_object.is_some_and(|object| {
                object.len() == 2
                    && object.get("type").and_then(Value::as_str)
                        == Some(self.schema.doc_node_type())
                    && expected_content.is_some()
            });
            let mut cursor = JsonMatchCursor::new(expected_content);
            for child in fragment.children(txn) {
                match_xml_out_json(
                    child,
                    txn,
                    2,
                    &mut cursor,
                    &mut matched,
                    &mut context,
                    Some(&mut collector),
                )?;
            }
            collector.end_container();
            cursor.finish(&mut matched);
            Ok(matched)
        })();
        let lookup = matched.as_ref().ok().and_then(|_| collector.finish().ok());
        (matched, lookup)
    }

    fn read_json_with_budget<T: ReadTxn>(
        &self,
        fragment: &XmlFragmentRef,
        txn: &T,
        mut budget: ConversionBudget<'_>,
        mut lookup: Option<&mut ImportLookupMaterializationCollector>,
    ) -> YrsEngineResult<serde_json::Value> {
        #[cfg(test)]
        JSON_PROJECTION_MATERIALIZATION_COUNT
            .with(|count| count.set(count.get().saturating_add(1)));
        budget.admit_node(1)?;
        let mut content = Vec::new();
        for child in fragment.children(txn) {
            append_xml_out_json(
                child,
                txn,
                self.schema,
                2,
                &mut budget,
                &mut content,
                lookup.as_deref_mut(),
            )?;
        }
        if let Some(lookup) = lookup {
            lookup.end_container();
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

#[derive(Debug, Clone)]
pub(crate) enum PreparedXmlNode {
    Text {
        runs: Vec<PreparedTextRun>,
    },
    Element {
        tag: String,
        attrs: Vec<(String, Any)>,
        children: Vec<PreparedXmlChild>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PreparedTextRun {
    pub(crate) index_utf16: u32,
    pub(crate) text: String,
    pub(crate) attrs: Attrs,
}

#[derive(Debug, Clone)]
pub(crate) struct PreparedXmlChild {
    pub(crate) index: u32,
    pub(crate) node: PreparedXmlNode,
}

#[derive(Debug, Clone)]
pub(crate) struct PreparedXmlBatch {
    pub(crate) nodes: Vec<PreparedXmlChild>,
    pub(crate) work: usize,
}

pub(crate) fn prepare_xml_nodes(
    nodes: &[Value],
    limits: &ResourceLimits,
    depth: usize,
) -> YrsEngineResult<PreparedXmlBatch> {
    let mut budget = ConversionBudget::new(limits);
    let mut prepared = Vec::with_capacity(nodes.len());
    for (index, node) in nodes.iter().enumerate() {
        prepared.push(PreparedXmlChild {
            index: u32::try_from(index).map_err(|_| {
                YrsEngineError::new(
                    "DOCUMENT_LIMIT_EXCEEDED",
                    "prepared XML child index exceeds u32",
                )
            })?,
            node: prepare_json_node(node, depth, &mut budget)?,
        });
    }
    let work = budget
        .nodes
        .checked_add(budget.any_work)
        .and_then(|work| work.checked_add(budget.output_bytes))
        .ok_or_else(|| {
            YrsEngineError::new("DOCUMENT_LIMIT_EXCEEDED", "prepared XML work overflow")
        })?;
    Ok(PreparedXmlBatch {
        nodes: prepared,
        work,
    })
}

pub(crate) fn insert_prepared_node<P: XmlFragment>(
    parent: &P,
    txn: &mut TransactionMut<'_>,
    index: u32,
    node: PreparedXmlNode,
) {
    stacker::maybe_grow(
        RECURSION_RED_ZONE_BYTES,
        RECURSION_STACK_SEGMENT_BYTES,
        || insert_prepared_node_inner(parent, txn, index, node),
    );
}

fn insert_prepared_node_inner<P: XmlFragment>(
    parent: &P,
    txn: &mut TransactionMut<'_>,
    index: u32,
    node: PreparedXmlNode,
) {
    match node {
        PreparedXmlNode::Text { runs } => {
            let target = parent.insert(txn, index, XmlTextPrelim::new(""));
            for run in runs {
                if !run.text.is_empty() {
                    target.insert_with_attributes(txn, run.index_utf16, &run.text, run.attrs);
                }
            }
        }
        PreparedXmlNode::Element {
            tag,
            attrs,
            children,
        } => {
            let element = parent.insert(txn, index, XmlElementPrelim::empty(tag.as_str()));
            for (key, value) in attrs {
                element.insert_attribute(txn, key.as_str(), value);
            }
            for child in children {
                insert_prepared_node(&element, txn, child.index, child.node);
            }
        }
    }
}

#[derive(Debug)]
struct ConversionBudget<'a> {
    limits: &'a ResourceLimits,
    nodes: usize,
    raw_text_runs: usize,
    any_work: usize,
    output_bytes: usize,
    charge_output: bool,
}

impl<'a> ConversionBudget<'a> {
    fn new(limits: &'a ResourceLimits) -> Self {
        Self {
            limits,
            nodes: 0,
            raw_text_runs: 0,
            any_work: 0,
            output_bytes: 0,
            charge_output: true,
        }
    }

    fn for_validated_source(limits: &'a ResourceLimits) -> Self {
        Self {
            charge_output: false,
            ..Self::new(limits)
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

    fn admit_raw_text_run(&mut self, depth: usize) -> YrsEngineResult<()> {
        self.admit_traversal_depth(depth)?;
        self.raw_text_runs = self.raw_text_runs.saturating_add(1);
        let limit = raw_storage_work_limit(self.limits);
        if self.raw_text_runs > limit {
            return Err(YrsEngineError::limit(
                "DOCUMENT_LIMIT_EXCEEDED",
                limit,
                self.raw_text_runs,
            )
            .with_details(json!({
                "phase": "candidateMaterialization",
                "dimension": "rawTextRuns"
            })));
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
        let limit = raw_storage_work_limit(self.limits);
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
        if !self.charge_output {
            return Ok(());
        }
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

    fn charge_computed_output(&mut self, bytes: impl FnOnce() -> usize) -> YrsEngineResult<()> {
        if !self.charge_output {
            return Ok(());
        }
        self.charge_output(bytes())
    }
}

include!("codec/projection_match.rs");

include!("codec/projection_write.rs");

include!("codec/mutation.rs");

pub(crate) fn normalized_wire_element_node_type<T: ReadTxn>(
    element: &XmlElementRef,
    txn: &T,
) -> String {
    let tag = element.tag().as_ref();
    if tag != "heading" {
        return tag.to_string();
    }
    let level = match element.get_attribute(txn, "level") {
        Some(yrs::Out::Any(Any::BigInt(value))) => u8::try_from(value).ok(),
        Some(yrs::Out::Any(Any::Number(value))) => (value.is_finite() && value.fract() == 0.0)
            .then(|| u8::try_from(value as i64).ok())
            .flatten(),
        Some(yrs::Out::Any(Any::String(value))) => {
            crate::serialize::parse_wire_heading_level_str(&value)
        }
        _ => None,
    };
    level
        .filter(|level| (1..=6).contains(level))
        .map_or_else(|| tag.to_string(), |level| format!("h{level}"))
}

pub(crate) struct WireAttributeJsonBudget<'a> {
    budget: ConversionBudget<'a>,
}

impl<'a> WireAttributeJsonBudget<'a> {
    pub(crate) fn new(limits: &'a ResourceLimits) -> Self {
        Self {
            budget: ConversionBudget::new(limits),
        }
    }

    pub(crate) fn convert(&mut self, value: &Any) -> YrsEngineResult<Value> {
        any_to_json(value, &mut self.budget, 1)
    }
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
    stacker::maybe_grow(
        RECURSION_RED_ZONE_BYTES,
        RECURSION_STACK_SEGMENT_BYTES,
        || json_to_any_inner(value),
    )
}

fn json_to_any_inner(value: &Value) -> Any {
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
    stacker::maybe_grow(
        RECURSION_RED_ZONE_BYTES,
        RECURSION_STACK_SEGMENT_BYTES,
        || any_to_json_inner(value, budget, depth),
    )
}

fn any_to_json_inner(
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
            budget.charge_computed_output(|| {
                number.as_ref().map_or(4, |number| number.to_string().len())
            })?;
            Ok(number.map(Value::Number).unwrap_or(Value::Null))
        }
        Any::BigInt(value) => {
            budget.charge_computed_output(|| value.to_string().len())?;
            Ok(Value::Number((*value).into()))
        }
        Any::String(value) => {
            budget.charge_computed_output(|| json_string_len(value))?;
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
                budget.charge_computed_output(|| json_string_len(key).saturating_add(1))?;
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
#[path = "codec_tests.rs"]
mod tests;
