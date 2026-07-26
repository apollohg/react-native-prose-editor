use std::collections::HashMap;

use serde::Serialize;

use crate::editor_state::{ActiveState, HistoryState};
use crate::ffi_v2::types::decimal_u64;
use crate::model::{Fragment, Mark, Node};
use crate::render::incremental::RenderBlocksPatch;
use crate::render::RenderElement;

use super::TransactionOrigin;

pub type OperationResult<T> = Result<T, OperationError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorOffsetKind {
    Scalar,
    Utf16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Affinity {
    Before,
    After,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RevisionedPosition {
    pub offset: u32,
    pub kind: EditorOffsetKind,
    pub affinity: Affinity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RevisionedRange {
    pub from: RevisionedPosition,
    pub to: RevisionedPosition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedPoint {
    pub document: u32,
    pub scalar: u32,
    pub utf16: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedSelection {
    Text {
        anchor: ResolvedPoint,
        head: ResolvedPoint,
    },
    Node {
        at: ResolvedPoint,
    },
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryPolicy {
    Auto,
    Boundary,
    Skip,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionIntent {
    Preserve,
    Set(SelectionInput),
    UseOperationResult,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionInput {
    Text {
        anchor: RevisionedPosition,
        head: RevisionedPosition,
    },
    Node {
        at: RevisionedPosition,
    },
    All,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypedTransaction {
    pub request_id: u64,
    pub base_document_revision: u64,
    pub origin: TransactionOrigin,
    pub operations: Vec<TypedOperation>,
    pub selection_intent: SelectionIntent,
    pub history_policy: HistoryPolicy,
}

#[derive(Debug, Clone, PartialEq)]
/// A deterministic same-parent structural child-window replacement.
///
/// The payload is inspectable but crate-sealed: external callers cannot forge
/// structural targets or their policy.
///
/// ```compile_fail
/// use editor_core::yrs_engine::StructuralReplacement;
/// let _ = StructuralReplacement::new(
///     vec![], 0, 0, Default::default(), editor_core::selection::Selection::cursor(0)
/// );
/// ```
pub struct StructuralReplacement {
    parent_path: Vec<u32>,
    from_child: u32,
    to_child: u32,
    content: Fragment,
    selection_after: crate::selection::Selection,
}

impl StructuralReplacement {
    pub(crate) fn new(
        parent_path: Vec<u32>,
        from_child: u32,
        to_child: u32,
        content: Fragment,
        selection_after: crate::selection::Selection,
    ) -> Self {
        Self {
            parent_path,
            from_child,
            to_child,
            content,
            selection_after,
        }
    }

    pub fn parent_path(&self) -> &[u32] {
        &self.parent_path
    }

    pub fn child_window(&self) -> (u32, u32) {
        (self.from_child, self.to_child)
    }

    pub fn content(&self) -> &Fragment {
        &self.content
    }

    pub(crate) fn selection_after(&self) -> &crate::selection::Selection {
        &self.selection_after
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypedOperation {
    ReplaceStructure(StructuralReplacement),
    InsertText {
        at: RevisionedPosition,
        text: String,
        marks: Vec<Mark>,
    },
    DeleteRange {
        range: RevisionedRange,
    },
    ReplaceRange {
        range: RevisionedRange,
        content: Fragment,
    },
    AddMark {
        range: RevisionedRange,
        mark: Mark,
    },
    RemoveMark {
        range: RevisionedRange,
        mark_type: String,
    },
    ReplaceMark {
        range: RevisionedRange,
        mark: Mark,
    },
    SplitBlock {
        at: RevisionedPosition,
        node_type: String,
        attrs: HashMap<String, serde_json::Value>,
    },
    JoinBlocks {
        at: RevisionedPosition,
    },
    WrapInList {
        range: RevisionedRange,
        list_type: String,
        item_type: String,
        attrs: HashMap<String, serde_json::Value>,
        item_attrs: HashMap<String, serde_json::Value>,
    },
    UnwrapFromList {
        at: RevisionedPosition,
    },
    IndentListItem {
        at: RevisionedPosition,
    },
    OutdentListItem {
        at: RevisionedPosition,
    },
    InsertNode {
        at: RevisionedPosition,
        node: Node,
    },
    UpdateNodeAttrs {
        at: RevisionedPosition,
        attrs: HashMap<String, serde_json::Value>,
    },
}

/// History class of a same-store whole-document root replacement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplacementHistory {
    /// `setContent` / `setContentJson` / replace-mode controlled value:
    /// exactly one undoable local-API history boundary.
    UndoableBoundary,
    /// Reset/import mode: non-undoable, clears the local history.
    ResetAndClear,
}

/// Failure surface of a same-store whole-document root replacement.
///
/// `Admission` carries the exact import-pipeline error (bounded input, parse,
/// model/schema validation, canonical-output ceilings); `Transaction` carries
/// the sealed root-window transaction rejection. Both stages reject atomically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RootReplacementError {
    Admission(super::YrsEngineError),
    Transaction(OperationError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransactionCommit {
    pub request_id: u64,
    pub changed: bool,
    pub document_revision: u64,
    pub state_revision: u64,
    pub origin: TransactionOrigin,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RenderUpdate {
    None,
    Patch(RenderBlocksPatch),
    Full(Vec<Vec<RenderElement>>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypedTransactionResult {
    pub request_id: u64,
    pub origin: TransactionOrigin,
    pub changed: bool,
    pub document_revision: u64,
    pub state_revision: u64,
    pub selection: ResolvedSelection,
    pub active_state: ActiveState,
    pub history_state: HistoryState,
    pub render_update: RenderUpdate,
}

impl TypedTransactionResult {
    /// Deterministic allocation/payload charge used by pre-write admission.
    pub fn derived_output_bytes(&self) -> usize {
        let mut bytes = 8usize
            .saturating_add(1)
            .saturating_add(1)
            .saturating_add(8)
            .saturating_add(8);
        bytes = bytes.saturating_add(selection_bytes(&self.selection));
        bytes = bytes.saturating_add(active_state_bytes(&self.active_state));
        bytes = bytes.saturating_add(2);
        bytes.saturating_add(render_update_bytes(&self.render_update))
    }
}

impl From<&TypedTransactionResult> for TransactionCommit {
    fn from(result: &TypedTransactionResult) -> Self {
        Self {
            request_id: result.request_id,
            changed: result.changed,
            document_revision: result.document_revision,
            state_revision: result.state_revision,
            origin: result.origin,
        }
    }
}

fn selection_bytes(selection: &ResolvedSelection) -> usize {
    match selection {
        ResolvedSelection::Text { .. } => 1usize.saturating_add(6 * 4),
        ResolvedSelection::Node { .. } => 1usize.saturating_add(3 * 4),
        ResolvedSelection::All => 1,
    }
}

pub(crate) fn active_state_bytes(state: &ActiveState) -> usize {
    let bool_map = |map: &HashMap<String, bool>| {
        map.iter().fold(8usize, |bytes, (key, _)| {
            bytes.saturating_add(string_bytes(key)).saturating_add(1)
        })
    };
    let attrs = state.mark_attrs.iter().fold(8usize, |bytes, (key, value)| {
        bytes
            .saturating_add(string_bytes(key))
            .saturating_add(json_bytes(value))
    });
    let strings = |values: &[String]| {
        values.iter().fold(8usize, |bytes, value| {
            bytes.saturating_add(string_bytes(value))
        })
    };
    bool_map(&state.marks)
        .saturating_add(attrs)
        .saturating_add(bool_map(&state.nodes))
        .saturating_add(bool_map(&state.commands))
        .saturating_add(strings(&state.allowed_marks))
        .saturating_add(strings(&state.insertable_nodes))
}

fn render_update_bytes(update: &RenderUpdate) -> usize {
    match update {
        RenderUpdate::None => 1,
        RenderUpdate::Patch(patch) => 1usize
            .saturating_add(16)
            .saturating_add(render_blocks_bytes(&patch.blocks)),
        RenderUpdate::Full(blocks) => 1usize.saturating_add(render_blocks_bytes(blocks)),
    }
}

fn render_blocks_bytes(blocks: &[Vec<RenderElement>]) -> usize {
    blocks.iter().fold(8usize, |bytes, block| {
        block
            .iter()
            .fold(bytes.saturating_add(8), |bytes, element| {
                bytes.saturating_add(render_element_bytes(element))
            })
    })
}

fn attrs_bytes(attrs: &HashMap<String, serde_json::Value>) -> usize {
    8usize.saturating_add(serde_json::to_vec(attrs).map_or(usize::MAX, |value| value.len()))
}

fn json_bytes(value: &serde_json::Value) -> usize {
    8usize.saturating_add(serde_json::to_vec(value).map_or(usize::MAX, |value| value.len()))
}

fn string_bytes(value: &str) -> usize {
    8usize.saturating_add(value.len())
}

fn render_element_bytes(element: &RenderElement) -> usize {
    let payload = match element {
        RenderElement::TextRun { text, marks } => {
            marks
                .iter()
                .fold(string_bytes(text).saturating_add(8), |bytes, mark| {
                    bytes
                        .saturating_add(string_bytes(&mark.mark_type))
                        .saturating_add(attrs_bytes(&mark.attrs))
                })
        }
        RenderElement::VoidInline {
            node_type, attrs, ..
        }
        | RenderElement::VoidBlock {
            node_type, attrs, ..
        } => string_bytes(node_type)
            .saturating_add(4)
            .saturating_add(attrs_bytes(attrs)),
        RenderElement::OpaqueInlineAtom {
            node_type,
            label,
            attrs,
            mention_theme,
            ..
        } => string_bytes(node_type)
            .saturating_add(string_bytes(label))
            .saturating_add(4)
            .saturating_add(1)
            .saturating_add(attrs_bytes(attrs))
            .saturating_add(mention_theme.as_ref().map_or(0, attrs_bytes)),
        RenderElement::OpaqueBlockAtom {
            node_type, label, attrs, ..
        } => string_bytes(node_type)
            .saturating_add(string_bytes(label))
            .saturating_add(4)
            .saturating_add(attrs_bytes(attrs)),
        RenderElement::BlockStart {
            node_type,
            list_context,
            ..
        } => string_bytes(node_type)
            .saturating_add(2)
            .saturating_add(1)
            .saturating_add(list_context.as_ref().map_or(0, |context| {
                18usize.saturating_add(context.kind.as_ref().map_or(0, |kind| string_bytes(kind)))
            })),
        RenderElement::BlockEnd => 0,
    };
    1usize.saturating_add(payload)
}

#[cfg(test)]
mod result_meter_tests {
    use super::*;
    use crate::render::RenderMark;

    #[test]
    fn result_meter_charges_nested_containers_fields_and_attribute_payloads() {
        let attrs = HashMap::from([("href".to_string(), serde_json::json!("x"))]);
        let result = TypedTransactionResult {
            request_id: 1,
            origin: TransactionOrigin::LocalApi,
            changed: true,
            document_revision: 1,
            state_revision: 1,
            selection: ResolvedSelection::Text {
                anchor: ResolvedPoint {
                    document: 1,
                    scalar: 0,
                    utf16: 0,
                },
                head: ResolvedPoint {
                    document: 1,
                    scalar: 0,
                    utf16: 0,
                },
            },
            active_state: ActiveState {
                marks: HashMap::from([("bold".into(), true)]),
                mark_attrs: HashMap::from([(
                    "bold".into(),
                    serde_json::Value::Object(attrs.clone().into_iter().collect()),
                )]),
                nodes: HashMap::new(),
                commands: HashMap::new(),
                allowed_marks: vec!["bold".into()],
                insertable_nodes: vec![],
            },
            history_state: HistoryState {
                can_undo: true,
                can_redo: false,
            },
            render_update: RenderUpdate::Full(vec![vec![RenderElement::TextRun {
                text: "x".into(),
                marks: vec![RenderMark {
                    mark_type: "bold".into(),
                    attrs,
                }],
            }]]),
        };

        assert_eq!(result.derived_output_bytes(), 225);
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OperationError {
    pub code: &'static str,
    pub message: Box<str>,
    pub request_id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl OperationError {
    pub fn engine_not_ready(request_id: u64) -> Self {
        Self::new(
            "ENGINE_NOT_READY",
            "the document engine is not ready",
            request_id,
        )
    }

    pub fn revision_mismatch(
        request_id: u64,
        expected_revision: u64,
        actual_revision: u64,
    ) -> Self {
        Self::new(
            "REVISION_MISMATCH",
            format!(
                "document revision mismatch: expected {expected_revision}, actual {actual_revision}"
            ),
            request_id,
        )
        .with_details(serde_json::json!({
            "expectedRevision": decimal_u64(expected_revision),
            "actualRevision": decimal_u64(actual_revision),
        }))
    }

    pub fn position_invalid(
        request_id: u64,
        operation_index: usize,
        field: &'static str,
        message: impl Into<String>,
    ) -> Self {
        Self::position_invalid_at(request_id, Some(operation_index), field, message)
    }

    pub(crate) fn selection_position_invalid(
        request_id: u64,
        field: &'static str,
        message: impl Into<String>,
    ) -> Self {
        Self::position_invalid_at(request_id, None, field, message)
    }

    pub fn transaction_invalid(
        request_id: u64,
        field: &'static str,
        message: impl Into<String>,
    ) -> Self {
        Self::new("TRANSACTION_INVALID", message, request_id)
            .with_details(serde_json::json!({ "field": field }))
    }

    pub fn operation_invalid(
        request_id: u64,
        operation_index: usize,
        field: &'static str,
        message: impl Into<String>,
    ) -> Self {
        Self::new("OPERATION_INVALID", message, request_id)
            .at_operation(operation_index)
            .with_details(serde_json::json!({ "field": field }))
    }

    pub fn operation_limit_exceeded(
        request_id: u64,
        operation_index: Option<usize>,
        field: &'static str,
        limit: u64,
        actual: u64,
    ) -> Self {
        Self::limit(
            "OPERATION_LIMIT_EXCEEDED",
            request_id,
            operation_index,
            field,
            limit,
            actual,
        )
    }

    pub fn document_invalid(
        request_id: u64,
        operation_index: Option<usize>,
        field: &'static str,
        message: impl Into<String>,
    ) -> Self {
        let error = Self::new("DOCUMENT_INVALID", message, request_id)
            .with_details(serde_json::json!({ "field": field }));
        error.with_operation_index(operation_index)
    }

    pub fn document_limit_exceeded(
        request_id: u64,
        operation_index: Option<usize>,
        field: &'static str,
        limit: u64,
        actual: u64,
    ) -> Self {
        Self::limit(
            "DOCUMENT_LIMIT_EXCEEDED",
            request_id,
            operation_index,
            field,
            limit,
            actual,
        )
    }

    pub fn engine_invariant_failed(
        request_id: u64,
        operation_index: Option<usize>,
        message: impl Into<String>,
    ) -> Self {
        Self::new("ENGINE_INVARIANT_FAILED", message, request_id)
            .with_operation_index(operation_index)
    }

    pub(crate) fn operation_resource_exhausted(
        request_id: u64,
        field: &'static str,
        message: impl Into<String>,
    ) -> Self {
        Self::new("OPERATION_RESOURCE_EXHAUSTED", message, request_id)
            .with_details(serde_json::json!({ "field": field }))
    }

    /// Deterministic bounded-work exhaustion (Task 16B): the work budget is a
    /// derived ceiling, so no meaningful limit/actual pair is reported —
    /// unlike `operation_limit_exceeded`, which carries both.
    pub(crate) fn operation_work_budget_exceeded(
        request_id: u64,
        field: &'static str,
        message: impl Into<String>,
    ) -> Self {
        Self::new("OPERATION_LIMIT_EXCEEDED", message, request_id)
            .with_details(serde_json::json!({ "field": field }))
    }

    pub(crate) fn revision_overflow(request_id: u64, field: &'static str) -> Self {
        Self::engine_invariant_failed(request_id, None, format!("{field} cannot be incremented"))
            .with_details(serde_json::json!({ "field": field }))
    }

    #[cfg(test)]
    pub(crate) fn atomic_failpoint(request_id: u64, failpoint: &'static str) -> Self {
        Self::engine_invariant_failed(request_id, None, format!("atomic failpoint: {failpoint}"))
            .with_details(serde_json::json!({ "failpoint": failpoint }))
    }

    fn new(code: &'static str, message: impl Into<String>, request_id: u64) -> Self {
        Self {
            code,
            message: message.into().into_boxed_str(),
            request_id,
            operation_index: None,
            limit: None,
            actual: None,
            details: None,
        }
    }

    fn position_invalid_at(
        request_id: u64,
        operation_index: Option<usize>,
        field: &'static str,
        message: impl Into<String>,
    ) -> Self {
        Self::new("POSITION_INVALID", message, request_id)
            .with_operation_index(operation_index)
            .with_details(serde_json::json!({ "field": field }))
    }

    fn limit(
        code: &'static str,
        request_id: u64,
        operation_index: Option<usize>,
        field: &'static str,
        limit: u64,
        actual: u64,
    ) -> Self {
        Self::new(
            code,
            format!("{field} exceeds limit {limit}: {actual}"),
            request_id,
        )
        .with_operation_index(operation_index)
        .with_limit(limit, actual)
        .with_details(serde_json::json!({ "field": field }))
    }

    fn at_operation(self, operation_index: usize) -> Self {
        self.with_operation_index(Some(operation_index))
    }

    fn with_operation_index(mut self, operation_index: Option<usize>) -> Self {
        self.operation_index = operation_index;
        self
    }

    fn with_limit(mut self, limit: u64, actual: u64) -> Self {
        self.limit = Some(limit);
        self.actual = Some(actual);
        self
    }

    fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }
}

impl std::fmt::Display for OperationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for OperationError {}
