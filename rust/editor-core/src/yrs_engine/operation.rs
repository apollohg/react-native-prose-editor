use std::collections::HashMap;

use serde::Serialize;

use crate::model::{Fragment, Mark, Node};

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
pub enum TypedOperation {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransactionCommit {
    pub request_id: u64,
    pub changed: bool,
    pub document_revision: u64,
    pub state_revision: u64,
    pub origin: TransactionOrigin,
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
            "expectedRevision": expected_revision,
            "actualRevision": actual_revision,
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
