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
use crate::schema::{NodeRole, Schema};

use super::mutation::{
    ImportElementAttributeWork, ImportLookupMaterializationCollector, ImportTextCaptureWork,
};
use super::{raw_storage_work_limit, YrsEngineError, YrsEngineResult};

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
        Some(yrs::Out::Any(Any::BigInt(value))) => u8::try_from(value).ok(),
        Some(yrs::Out::Any(Any::Number(value))) => (value.is_finite() && value.fract() == 0.0)
            .then(|| u8::try_from(value as i64).ok())
            .flatten(),
        Some(yrs::Out::Any(Any::String(value))) => {
            crate::serialize::parse_wire_heading_level_str(&value)
        }
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
    let (node_type, removes_level) = normalized_type(tag.as_ref(), level);
    let mut local_match = expected_object
        .is_some_and(|object| object.get("type").and_then(Value::as_str) == Some(node_type));
    let collect_lookup = lookup.is_some();
    let mut lookup_attribute_work = ImportElementAttributeWork::new();
    let mut projected_attr_count = 0usize;
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
        if removes_level && key == "level" {
            continue;
        }
        projected_attr_count = projected_attr_count.saturating_add(1);
        local_match &= any_matches_json(&value, expected_attrs.and_then(|attrs| attrs.get(key)));
    }
    local_match &= expected_attrs.map_or(0, Map::len) == projected_attr_count;

    let (is_void, is_textblock) = context
        .schema
        .node(node_type)
        .map_or((true, false), |spec| {
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

fn append_xml_out_json<T: ReadTxn>(
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
    let node_type = normalized_wire_element_node_type(element, txn);
    let (is_void, is_textblock) = schema.node(&node_type).map_or((true, false), |spec| {
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
    if node_type != element.tag().as_ref() {
        attrs.remove("level");
    }
    object.insert("type".to_string(), Value::String(node_type));
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
    let prepared = prepare_json_node(node, depth, budget)?;
    insert_prepared_node(parent, txn, index, prepared);
    Ok(())
}

fn prepare_json_node(
    node: &Value,
    depth: usize,
    budget: &mut ConversionBudget<'_>,
) -> YrsEngineResult<PreparedXmlNode> {
    budget.admit_node(depth)?;
    let node_type = node.get("type").and_then(Value::as_str).unwrap_or("");
    if node_type == "text" {
        let text_value = node.get("text").and_then(Value::as_str).unwrap_or("");
        budget.charge_output(text_value.len())?;
        return Ok(PreparedXmlNode::Text {
            runs: vec![PreparedTextRun {
                index_utf16: 0,
                text: text_value.to_owned(),
                attrs: prepare_marks_to_attrs(node.get("marks").and_then(Value::as_array), budget)?,
            }],
        });
    }

    let mut attrs = node
        .get("attrs")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let element_name = element_name_for_json_node(node_type, &mut attrs);
    let mut prepared_attrs = Vec::with_capacity(attrs.len());
    let mut sorted_attrs = attrs.iter().collect::<Vec<_>>();
    sorted_attrs.sort_by_key(|(key, _)| *key);
    for (key, value) in sorted_attrs {
        budget.charge_output(key.len())?;
        prepared_attrs.push((key.clone(), prepare_json_value(value, budget, 1)?));
    }
    let mut prepared_children = Vec::new();
    if let Some(children) = node.get("content").and_then(Value::as_array) {
        prepared_children.reserve(children.len());
        for (index, child) in children.iter().enumerate() {
            prepared_children.push(PreparedXmlChild {
                index: u32::try_from(index).map_err(|_| {
                    YrsEngineError::new(
                        "DOCUMENT_LIMIT_EXCEEDED",
                        "prepared XML child index exceeds u32",
                    )
                })?,
                node: prepare_json_node(child, depth + 1, budget)?,
            });
        }
    }
    Ok(PreparedXmlNode::Element {
        tag: element_name,
        attrs: prepared_attrs,
        children: prepared_children,
    })
}

fn prepare_marks_to_attrs(
    marks: Option<&Vec<Value>>,
    budget: &mut ConversionBudget<'_>,
) -> YrsEngineResult<Attrs> {
    let mut attrs = Attrs::default();
    let Some(marks) = marks else {
        return Ok(attrs);
    };
    for mark in marks {
        let Some(mark_type) = mark.get("type").and_then(Value::as_str) else {
            continue;
        };
        budget.charge_output(mark_type.len())?;
        let value = match mark.get("attrs") {
            Some(value) => prepare_json_value(value, budget, 1)?,
            None => {
                budget.admit_any(1, 1)?;
                Any::Bool(true)
            }
        };
        attrs.insert(mark_type.into(), value);
    }
    Ok(attrs)
}

fn prepare_json_value(
    value: &Value,
    budget: &mut ConversionBudget<'_>,
    depth: usize,
) -> YrsEngineResult<Any> {
    budget.admit_any(depth, 1)?;
    match value {
        Value::Null => Ok(Any::Null),
        Value::Bool(value) => Ok(Any::Bool(*value)),
        Value::Number(number) => {
            if let Some(value) = number.as_i64() {
                Ok(Any::BigInt(value))
            } else if let Some(value) = number.as_u64() {
                Err(YrsEngineError::new(
                    "DOCUMENT_INVALID",
                    format!("numeric attribute {value} exceeds the exact Yjs integer range"),
                ))
            } else if let Some(value) = number.as_f64() {
                if value.is_finite() {
                    Ok(Any::Number(value))
                } else {
                    Err(YrsEngineError::new(
                        "DOCUMENT_INVALID",
                        "numeric attribute must be finite",
                    ))
                }
            } else {
                Err(YrsEngineError::new(
                    "DOCUMENT_INVALID",
                    "numeric attribute is not representable",
                ))
            }
        }
        Value::String(value) => {
            budget.admit_any(depth, value.len())?;
            budget.charge_output(value.len())?;
            Ok(Any::String(value.clone().into()))
        }
        Value::Array(values) => {
            budget.admit_any(depth, values.len())?;
            let mut prepared = Vec::with_capacity(values.len());
            for value in values {
                prepared.push(prepare_json_value(value, budget, depth + 1)?);
            }
            Ok(Any::Array(prepared.into()))
        }
        Value::Object(values) => {
            budget.admit_any(depth, values.len())?;
            let mut prepared = std::collections::HashMap::with_capacity(values.len());
            let mut entries = values.iter().collect::<Vec<_>>();
            entries.sort_by_key(|(key, _)| *key);
            for (key, value) in entries {
                budget.admit_any(depth, key.len())?;
                budget.charge_output(key.len())?;
                prepared.insert(key.clone(), prepare_json_value(value, budget, depth + 1)?);
            }
            Ok(Any::Map(Arc::new(prepared)))
        }
    }
}

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
mod tests {
    use super::{
        actual_marks_equal, any_to_json_bounded, attrs_to_marks, insert_prepared_node,
        marks_to_attrs, prepare_xml_nodes, take_json_projection_materialization_count_for_test,
        YrsDocumentCodec,
    };
    use crate::boundary::ResourceLimits;
    use crate::schema::presets::tiptap_schema;
    use serde_json::{json, Value};
    use yrs::types::text::{Text, YChange};
    use yrs::types::xml::{
        Xml, XmlElementPrelim, XmlFragment, XmlFragmentPrelim, XmlIn, XmlTextPrelim,
    };
    use yrs::types::Attrs;
    use yrs::{Any, ArrayPrelim};
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

    fn matches_round_trip(next: &Value) -> (bool, bool) {
        let schema = tiptap_schema();
        let limits = ResourceLimits::default();
        let codec = YrsDocumentCodec::new(&schema, &limits);
        let doc = utf16_doc();
        {
            let mut txn = doc.transact_mut();
            let fragment = txn.get_or_insert_xml_fragment("prosemirror");
            codec
                .apply_json(&fragment, &mut txn, &empty_json("doc"), next)
                .unwrap();
        }
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let (matches, lookup) = codec.matches_validated_json_with_lookup(&fragment, &txn, next);
        (matches.unwrap(), lookup.is_some())
    }

    fn match_raw(
        doc: &Doc,
        expected: &Value,
        limits: &ResourceLimits,
    ) -> (super::YrsEngineResult<bool>, bool) {
        let schema = tiptap_schema();
        let codec = YrsDocumentCodec::new(&schema, limits);
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let (matched, lookup) = codec.matches_validated_json_with_lookup(&fragment, &txn, expected);
        (matched, lookup.is_some())
    }

    #[test]
    fn validated_json_matcher_avoids_old_value_projection() {
        let input = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "attrs": { "nested": [true, null, { "emoji": "😀" }] },
                "content": [{
                    "type": "text",
                    "text": "A😀e\u{301}",
                    "marks": [
                        { "type": "bold" },
                        { "type": "link", "attrs": { "href": "https://example.test/😀" } }
                    ]
                }]
            }, {
                "type": "__opaque_json",
                "attrs": {
                    "original_type": "callout",
                    "opaque_placement": "block",
                    "original_json": { "type": "callout", "attrs": { "rank": 7 } }
                }
            }]
        });

        take_json_projection_materialization_count_for_test();
        assert_eq!(matches_round_trip(&input), (true, true));
        assert_eq!(take_json_projection_materialization_count_for_test(), 0);

        let schema = tiptap_schema();
        let limits = ResourceLimits::default();
        let codec = YrsDocumentCodec::new(&schema, &limits);
        let doc = utf16_doc();
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror");
        assert!(fragment.is_none());
        drop(txn);
        let mut txn = doc.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        drop(txn);
        let txn = doc.transact();
        codec.read_json(&fragment, &txn).unwrap();
        assert_eq!(take_json_projection_materialization_count_for_test(), 1);
    }

    #[test]
    fn validated_json_matcher_coalesces_text_across_diffs_nodes_and_fragments() {
        let doc = utf16_doc();
        {
            let mut txn = doc.transact_mut();
            let fragment = txn.get_or_insert_xml_fragment("prosemirror");
            let heading = fragment.push_back(&mut txn, XmlElementPrelim::empty("heading"));
            heading.insert_attribute(&mut txn, "level", 2_i64);
            heading.push_back(&mut txn, XmlTextPrelim::new("A"));
            let nested = XmlFragmentPrelim::new::<_, XmlIn>([
                XmlIn::from(XmlTextPrelim::new("")),
                XmlIn::from(XmlTextPrelim::new("😀")),
            ]);
            heading.push_back(&mut txn, XmlIn::from(nested));
            let tail = heading.push_back(&mut txn, XmlTextPrelim::new(""));
            tail.insert_with_attributes(&mut txn, 0, "e\u{301}", Attrs::default());
        }
        let expected = json!({
            "type": "doc",
            "content": [{
                "type": "h2",
                "content": [{ "type": "text", "text": "A😀e\u{301}" }]
            }]
        });
        assert_eq!(
            match_raw(&doc, &expected, &ResourceLimits::default()),
            (Ok(true), true)
        );

        for mismatched in [
            json!({ "type": "doc", "content": [{ "type": "h3", "content": [{ "type": "text", "text": "A😀e\u{301}" }] }] }),
            json!({ "type": "doc", "content": [{ "type": "h2", "content": [{ "type": "text", "text": "different" }] }] }),
            json!({ "type": "doc", "content": [{ "type": "h2", "content": [{ "type": "text", "text": "A😀e\u{301}", "marks": [] }] }] }),
        ] {
            assert_eq!(
                match_raw(&doc, &mismatched, &ResourceLimits::default()).0,
                Ok(false)
            );
        }

        let null_projected_marks = utf16_doc();
        {
            let mut txn = null_projected_marks.transact_mut();
            let fragment = txn.get_or_insert_xml_fragment("prosemirror");
            let paragraph = fragment.push_back(&mut txn, XmlElementPrelim::empty("paragraph"));
            let first = paragraph.push_back(&mut txn, XmlTextPrelim::new(""));
            first.insert_with_attributes(
                &mut txn,
                0,
                "a",
                mark_attrs_value("custom", Any::Bool(true)),
            );
            let second = paragraph.push_back(&mut txn, XmlTextPrelim::new(""));
            second.insert_with_attributes(
                &mut txn,
                0,
                "b",
                mark_attrs_value("custom", Any::Number(f64::NAN)),
            );
        }
        let expected = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{
                    "type": "text",
                    "text": "ab",
                    "marks": [{ "type": "custom" }]
                }]
            }]
        });
        assert_eq!(read_raw(&null_projected_marks), expected);
        assert_eq!(
            match_raw(&null_projected_marks, &expected, &ResourceLimits::default()),
            (Ok(true), true)
        );

        let cross_variant_marks = utf16_doc();
        {
            let mut txn = cross_variant_marks.transact_mut();
            let fragment = txn.get_or_insert_xml_fragment("prosemirror");
            let paragraph = fragment.push_back(&mut txn, XmlElementPrelim::empty("paragraph"));
            let first = paragraph.push_back(&mut txn, XmlTextPrelim::new(""));
            first.insert_with_attributes(
                &mut txn,
                0,
                "a",
                mark_attrs_value("custom", Any::Buffer(vec![1, 2].into())),
            );
            let second = paragraph.push_back(&mut txn, XmlTextPrelim::new(""));
            second.insert_with_attributes(
                &mut txn,
                0,
                "b",
                mark_attrs_value(
                    "custom",
                    Any::Array(vec![Any::BigInt(1), Any::BigInt(2)].into()),
                ),
            );
        }
        let expected = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{
                    "type": "text",
                    "text": "ab",
                    "marks": [{ "type": "custom", "attrs": [1, 2] }]
                }]
            }]
        });
        assert_eq!(read_raw(&cross_variant_marks), expected);
        assert_eq!(
            match_raw(&cross_variant_marks, &expected, &ResourceLimits::default()),
            (Ok(true), true)
        );

        let empty_attrs_then_absent = utf16_doc();
        {
            let mut txn = empty_attrs_then_absent.transact_mut();
            let fragment = txn.get_or_insert_xml_fragment("prosemirror");
            let paragraph = fragment.push_back(&mut txn, XmlElementPrelim::empty("paragraph"));
            let first = paragraph.push_back(&mut txn, XmlTextPrelim::new(""));
            first.insert_with_attributes(&mut txn, 0, "a", Attrs::default());
            paragraph.push_back(&mut txn, XmlTextPrelim::new("b"));
        }
        let expected = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{ "type": "text", "text": "ab" }]
            }]
        });
        assert_eq!(read_raw(&empty_attrs_then_absent), expected);
        assert_eq!(
            match_raw(
                &empty_attrs_then_absent,
                &expected,
                &ResourceLimits::default()
            ),
            (Ok(true), true)
        );
    }

    #[test]
    fn validated_json_matcher_preserves_later_error_precedence_after_mismatch() {
        let malformed = utf16_doc();
        {
            let mut txn = malformed.transact_mut();
            let fragment = txn.get_or_insert_xml_fragment("prosemirror");
            let paragraph = fragment.push_back(&mut txn, XmlElementPrelim::empty("paragraph"));
            paragraph.push_back(&mut txn, XmlTextPrelim::new("first mismatch"));
            let invalid = paragraph.push_back(&mut txn, XmlTextPrelim::new(""));
            invalid.insert_embed_with_attributes(&mut txn, 0, Any::Bool(false), Attrs::default());
        }
        let wrong = json!({
            "type": "doc",
            "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "wrong" }] }]
        });
        let error = match_raw(&malformed, &wrong, &ResourceLimits::default())
            .0
            .unwrap_err();
        assert_eq!(error.code, "CODEC_INVARIANT_FAILED");
        assert_eq!(error.details.unwrap()["field"], "xmlTextRun");

        let fragmented = utf16_doc();
        {
            let mut txn = fragmented.transact_mut();
            let fragment = txn.get_or_insert_xml_fragment("prosemirror");
            let paragraph = fragment.push_back(&mut txn, XmlElementPrelim::empty("paragraph"));
            for _ in 0..385 {
                paragraph.push_back(&mut txn, XmlTextPrelim::new("x"));
            }
        }
        let error = match_raw(
            &fragmented,
            &wrong,
            &ResourceLimits {
                max_document_nodes: 3,
                ..ResourceLimits::default()
            },
        )
        .0
        .unwrap_err();
        assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
        assert_eq!(error.details.unwrap()["dimension"], "rawTextRuns");

        let distinct_text_nodes = utf16_doc();
        {
            let mut txn = distinct_text_nodes.transact_mut();
            let fragment = txn.get_or_insert_xml_fragment("prosemirror");
            let paragraph = fragment.push_back(&mut txn, XmlElementPrelim::empty("paragraph"));
            paragraph.push_back(&mut txn, XmlTextPrelim::new("a"));
            let bold = paragraph.push_back(&mut txn, XmlTextPrelim::new(""));
            bold.insert_with_attributes(&mut txn, 0, "b", mark_attrs("bold"));
            paragraph.push_back(&mut txn, XmlTextPrelim::new("c"));
        }
        let error = match_raw(
            &distinct_text_nodes,
            &wrong,
            &ResourceLimits {
                max_document_nodes: 4,
                ..ResourceLimits::default()
            },
        )
        .0
        .unwrap_err();
        assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
        assert_eq!(error.limit, Some(4));
        assert_eq!(error.actual, Some(5));

        let element_boundary = utf16_doc();
        {
            let mut txn = element_boundary.transact_mut();
            let fragment = txn.get_or_insert_xml_fragment("prosemirror");
            fragment.push_back(&mut txn, XmlTextPrelim::new("a"));
            fragment.push_back(&mut txn, XmlElementPrelim::empty("hardBreak"));
            fragment.push_back(&mut txn, XmlTextPrelim::new("b"));
        }
        let error = match_raw(
            &element_boundary,
            &json!({ "type": "wrong", "content": [] }),
            &ResourceLimits {
                max_document_nodes: 3,
                ..ResourceLimits::default()
            },
        )
        .0
        .unwrap_err();
        assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
        assert_eq!(error.limit, Some(3));
        assert_eq!(error.actual, Some(4));
    }

    #[test]
    fn validated_json_matcher_treats_lookup_collection_as_opportunistic() {
        use crate::yrs_engine::mutation::{
            set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
        };

        let input = json!({
            "type": "doc",
            "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "x" }] }]
        });
        let schema = tiptap_schema();
        let limits = ResourceLimits::default();
        let codec = YrsDocumentCodec::new(&schema, &limits);
        let doc = utf16_doc();
        {
            let mut txn = doc.transact_mut();
            let fragment = txn.get_or_insert_xml_fragment("prosemirror");
            codec
                .apply_json(&fragment, &mut txn, &empty_json("doc"), &input)
                .unwrap();
        }
        set_lookup_seed_hydration_failpoint_for_test(Some(
            LookupSeedHydrationFailpoint::InitialReservation,
        ));
        let result = match_raw(&doc, &input, &limits);
        set_lookup_seed_hydration_failpoint_for_test(None);
        assert_eq!(result, (Ok(true), false));
    }

    #[test]
    fn validated_json_matcher_projects_nonfinite_any_only_as_null() {
        let doc = utf16_doc();
        {
            let mut txn = doc.transact_mut();
            let fragment = txn.get_or_insert_xml_fragment("prosemirror");
            let paragraph = fragment.push_back(&mut txn, XmlElementPrelim::empty("paragraph"));
            paragraph.insert_attribute(&mut txn, "nan", f64::NAN);
            paragraph.insert_attribute(&mut txn, "positive", f64::INFINITY);
            paragraph.insert_attribute(&mut txn, "negative", f64::NEG_INFINITY);
        }
        let expected = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "attrs": { "nan": null, "positive": null, "negative": null }
            }]
        });
        assert_eq!(read_raw(&doc), expected);
        assert_eq!(
            match_raw(&doc, &expected, &ResourceLimits::default()),
            (Ok(true), true)
        );

        for wrong in [json!(false), json!("null"), json!([]), json!({})] {
            let mut mismatched = expected.clone();
            mismatched["content"][0]["attrs"]["nan"] = wrong;
            assert_eq!(
                match_raw(&doc, &mismatched, &ResourceLimits::default()).0,
                Ok(false)
            );
        }
    }

    fn read_raw(doc: &Doc) -> Value {
        let schema = tiptap_schema();
        let limits = ResourceLimits::default();
        let codec = YrsDocumentCodec::new(&schema, &limits);
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        codec.read_json(&fragment, &txn).unwrap()
    }

    fn mark_attrs(mark: &str) -> Attrs {
        mark_attrs_value(mark, Any::Bool(true))
    }

    fn mark_attrs_value(mark: &str, value: Any) -> Attrs {
        let mut attrs = Attrs::default();
        attrs.insert(mark.into(), value);
        attrs
    }

    #[test]
    fn read_json_coalesces_only_adjacent_equal_mark_text_storage() {
        let siblings = utf16_doc();
        {
            let mut txn = siblings.transact_mut();
            let fragment = txn.get_or_insert_xml_fragment("prosemirror");
            let paragraph = fragment.push_back(&mut txn, XmlElementPrelim::empty("paragraph"));
            paragraph.push_back(&mut txn, XmlTextPrelim::new("a"));
            paragraph.push_back(&mut txn, XmlTextPrelim::new("b"));
        }
        assert_eq!(
            read_raw(&siblings),
            json!({
                "type": "doc",
                "content": [{
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": "ab" }]
                }]
            })
        );

        let diff_runs = utf16_doc();
        let text = {
            let mut txn = diff_runs.transact_mut();
            let fragment = txn.get_or_insert_xml_fragment("prosemirror");
            let paragraph = fragment.push_back(&mut txn, XmlElementPrelim::empty("paragraph"));
            let text = paragraph.push_back(&mut txn, XmlTextPrelim::new(""));
            text.insert_with_attributes(&mut txn, 0, "ab", mark_attrs("bold"));
            text.insert_embed_with_attributes(&mut txn, 1, Any::Bool(false), Attrs::default());
            text
        };
        let txn = diff_runs.transact();
        assert_eq!(text.diff(&txn, YChange::identity).len(), 3);
        let schema = tiptap_schema();
        let limits = ResourceLimits::default();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let error = YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap_err();
        assert_eq!(error.code, "CODEC_INVARIANT_FAILED");
        assert_eq!(
            error.details,
            Some(json!({
                "phase": "candidateMaterialization",
                "field": "xmlTextRun"
            }))
        );
        drop(txn);

        let shared_embed = utf16_doc();
        {
            let mut txn = shared_embed.transact_mut();
            let fragment = txn.get_or_insert_xml_fragment("prosemirror");
            let paragraph = fragment.push_back(&mut txn, XmlElementPrelim::empty("paragraph"));
            let text = paragraph.push_back(&mut txn, XmlTextPrelim::new(""));
            text.insert_embed_with_attributes(
                &mut txn,
                0,
                ArrayPrelim::from(vec![Any::String("shared".into())]),
                Attrs::default(),
            );
        }
        let txn = shared_embed.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let error = YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap_err();
        assert_eq!(error.code, "CODEC_INVARIANT_FAILED");
        assert_eq!(
            error.details,
            Some(json!({
                "phase": "candidateMaterialization",
                "field": "xmlTextRun"
            }))
        );

        let different_marks = utf16_doc();
        {
            let mut txn = different_marks.transact_mut();
            let fragment = txn.get_or_insert_xml_fragment("prosemirror");
            let paragraph = fragment.push_back(&mut txn, XmlElementPrelim::empty("paragraph"));
            let bold = paragraph.push_back(&mut txn, XmlTextPrelim::new(""));
            bold.insert_with_attributes(&mut txn, 0, "a", mark_attrs("bold"));
            let italic = paragraph.push_back(&mut txn, XmlTextPrelim::new(""));
            italic.insert_with_attributes(&mut txn, 0, "b", mark_attrs("italic"));
        }
        assert_eq!(
            read_raw(&different_marks)["content"][0]["content"]
                .as_array()
                .unwrap()
                .len(),
            2
        );

        let element_boundary = utf16_doc();
        {
            let mut txn = element_boundary.transact_mut();
            let fragment = txn.get_or_insert_xml_fragment("prosemirror");
            let paragraph = fragment.push_back(&mut txn, XmlElementPrelim::empty("paragraph"));
            paragraph.push_back(&mut txn, XmlTextPrelim::new("a"));
            paragraph.push_back(&mut txn, XmlElementPrelim::empty("hardBreak"));
            paragraph.push_back(&mut txn, XmlTextPrelim::new("b"));
        }
        let element_json = read_raw(&element_boundary);
        assert_eq!(
            element_json["content"][0]["content"],
            json!([
                { "type": "text", "text": "a" },
                { "type": "hardBreak" },
                { "type": "text", "text": "b" }
            ])
        );

        let schema = tiptap_schema();
        let exact_limits = ResourceLimits {
            max_document_nodes: 3,
            ..ResourceLimits::default()
        };
        let txn = siblings.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        assert!(YrsDocumentCodec::new(&schema, &exact_limits)
            .read_json(&fragment, &txn)
            .is_ok());
        let rejected_limits = ResourceLimits {
            max_document_nodes: 2,
            ..ResourceLimits::default()
        };
        let error = YrsDocumentCodec::new(&schema, &rejected_limits)
            .read_json(&fragment, &txn)
            .unwrap_err();
        assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
        assert_eq!(error.limit, Some(2));
        assert_eq!(error.actual, Some(3));

        let raw_work_limits = ResourceLimits {
            max_document_nodes: 3,
            ..ResourceLimits::default()
        };
        let exact_fragmented = utf16_doc();
        {
            let mut txn = exact_fragmented.transact_mut();
            let fragment = txn.get_or_insert_xml_fragment("prosemirror");
            let paragraph = fragment.push_back(&mut txn, XmlElementPrelim::empty("paragraph"));
            for _ in 0..384 {
                paragraph.push_back(&mut txn, XmlTextPrelim::new("x"));
            }
        }
        let txn = exact_fragmented.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let exact_fragmented_json = YrsDocumentCodec::new(&schema, &raw_work_limits)
            .read_json(&fragment, &txn)
            .unwrap();
        assert_eq!(
            exact_fragmented_json["content"][0]["content"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            exact_fragmented_json["content"][0]["content"][0]["text"]
                .as_str()
                .unwrap()
                .chars()
                .count(),
            384
        );

        let fragmented = utf16_doc();
        {
            let mut txn = fragmented.transact_mut();
            let fragment = txn.get_or_insert_xml_fragment("prosemirror");
            let paragraph = fragment.push_back(&mut txn, XmlElementPrelim::empty("paragraph"));
            for _ in 0..385 {
                paragraph.push_back(&mut txn, XmlTextPrelim::new("x"));
            }
        }
        let txn = fragmented.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let error = YrsDocumentCodec::new(&schema, &raw_work_limits)
            .read_json(&fragment, &txn)
            .unwrap_err();
        assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
        assert_eq!(error.limit, Some(384));
        assert_eq!(error.actual, Some(385));
        assert_eq!(
            error.details,
            Some(json!({
                "phase": "candidateMaterialization",
                "dimension": "rawTextRuns"
            }))
        );
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
    fn prepared_builder_matches_codec_for_heading_void_and_nested_any() {
        let input = json!({
            "type": "doc",
            "content": [{
                "type": "h2",
                "attrs": {
                    "data": { "nested": [true, null, "😀", { "value": 7 }] }
                },
                "content": [{ "type": "text", "text": "title" }]
            }, {
                "type": "hardBreak",
                "attrs": { "meta": ["void", 2] }
            }, {
                "type": "image",
                "attrs": {
                    "src": "https://example.test/image.png",
                    "metadata": { "widths": [320, 640] }
                }
            }]
        });
        let schema = tiptap_schema();
        let limits = ResourceLimits::default();
        let codec = YrsDocumentCodec::new(&schema, &limits);

        let imported = utf16_doc();
        {
            let mut txn = imported.transact_mut();
            let fragment = txn.get_or_insert_xml_fragment("prosemirror");
            codec
                .apply_json(&fragment, &mut txn, &empty_json("doc"), &input)
                .unwrap();
        }
        let expected = {
            let txn = imported.transact();
            let fragment = txn.get_xml_fragment("prosemirror").unwrap();
            codec.read_json(&fragment, &txn).unwrap()
        };

        let prepared_doc = utf16_doc();
        {
            let mut txn = prepared_doc.transact_mut();
            let fragment = txn.get_or_insert_xml_fragment("prosemirror");
            let batch =
                prepare_xml_nodes(input["content"].as_array().unwrap(), &limits, 2).unwrap();
            for child in batch.nodes {
                insert_prepared_node(&fragment, &mut txn, child.index, child.node);
            }
        }
        let actual = {
            let txn = prepared_doc.transact();
            let fragment = txn.get_xml_fragment("prosemirror").unwrap();
            codec.read_json(&fragment, &txn).unwrap()
        };
        assert_eq!(actual, expected);

        let unsafe_number = json!({
            "type": "image",
            "attrs": { "unsafe": u64::MAX }
        });
        let error = prepare_xml_nodes(&[unsafe_number], &limits, 2).unwrap_err();
        assert_eq!(error.code, "DOCUMENT_INVALID");
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
        let empty = Attrs::default();
        assert!(actual_marks_equal(None, Some(&empty)));
        assert!(actual_marks_equal(Some(&empty), None));
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
                        "content": [
                            { "type": "text", "text": "preserve " },
                            { "type": "text", "text": "me" }
                        ]
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
