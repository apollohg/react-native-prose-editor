//! Serialization-ready editor commands and pure typed-transaction planning.

mod format;
mod structure;
mod text;

use std::collections::HashMap;

use crate::model::{Document, Mark};
use crate::position::PositionMap;
use crate::schema::Schema;
use crate::selection::Selection;

use super::{
    OperationResult, ResolvedSelection, RevisionedPosition, RevisionedRange, TypedTransaction,
};

#[derive(Debug, Clone, PartialEq)]
pub enum TypedCommand {
    InsertText {
        text: String,
    },
    DeleteRange {
        range: RevisionedRange,
    },
    DeleteBackward,
    ReplaceSelectionText {
        text: String,
    },
    SplitBlock,
    DeleteAndSplit,
    InsertContentJson {
        json: serde_json::Value,
    },
    InsertContentHtml {
        html: String,
    },
    ToggleMark {
        mark_type: String,
    },
    SetMark {
        mark_type: String,
        attrs: HashMap<String, serde_json::Value>,
    },
    UnsetMark {
        mark_type: String,
    },
    ToggleHeading {
        level: u8,
    },
    ToggleCodeBlock,
    ToggleBlockquote,
    ApplyListType {
        list_type: String,
    },
    WrapInList {
        list_type: String,
        item_type: String,
    },
    UnwrapFromList,
    IndentListItem,
    OutdentListItem,
    ToggleTaskItemChecked,
    InsertNode {
        node_type: String,
    },
    ResizeImage {
        at: RevisionedPosition,
        width: u32,
        height: u32,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum CommandPlan {
    NotApplicable,
    Transaction(TypedTransaction),
    SelectionOnly(TypedTransaction),
}

pub(crate) struct PreparedCommandProof {
    pub document: Document,
    pub selection: Selection,
    pub execution_admission: crate::yrs_engine::prepared_admission::ExecutionSemanticAdmission,
}

#[cfg(test)]
impl PreparedCommandProof {
    pub(crate) fn eager_semantic_admission_mut_for_test(
        &mut self,
    ) -> &mut crate::yrs_engine::compiler::PreparedSemanticAdmission {
        let crate::yrs_engine::prepared_admission::ExecutionSemanticAdmission::Eager(admission) =
            &mut self.execution_admission
        else {
            panic!("test requires an eager prepared semantic admission")
        };
        admission
    }
}

pub(crate) struct PlanningContext<'a> {
    pub request_id: u64,
    pub revision: u64,
    pub state_revision: u64,
    pub document: &'a Document,
    pub position_map: &'a PositionMap,
    pub rendered_text: &'a str,
    pub selection: &'a ResolvedSelection,
    pub stored_marks: Option<&'a [Mark]>,
    pub schema: &'a Schema,
    pub resource_limits: &'a crate::boundary::ResourceLimits,
    pub editing_limits: &'a crate::yrs_engine::EditingLimits,
    pub max_length: Option<u32>,
    pub yrs_state_epoch: u64,
    pub canonical_schema: &'a crate::yrs_engine::canonical::CanonicalSchemaContext,
    pub canonical_artifact: &'a crate::yrs_engine::canonical::CanonicalArtifact,
    pub allow_deferred_admission: bool,
    pub preparation: Option<&'a std::cell::RefCell<Option<PreparedCommandProof>>>,
}

pub(crate) fn plan(
    context: PlanningContext<'_>,
    command: TypedCommand,
) -> OperationResult<CommandPlan> {
    match command {
        command @ (TypedCommand::InsertText { .. }
        | TypedCommand::DeleteRange { .. }
        | TypedCommand::DeleteBackward
        | TypedCommand::ReplaceSelectionText { .. }
        | TypedCommand::SplitBlock
        | TypedCommand::DeleteAndSplit
        | TypedCommand::InsertContentJson { .. }
        | TypedCommand::InsertContentHtml { .. }) => text::plan(context, command),
        command @ (TypedCommand::ToggleMark { .. }
        | TypedCommand::SetMark { .. }
        | TypedCommand::UnsetMark { .. }
        | TypedCommand::ToggleHeading { .. }
        | TypedCommand::ToggleCodeBlock
        | TypedCommand::ToggleBlockquote) => format::plan(context, command),
        command @ (TypedCommand::ApplyListType { .. }
        | TypedCommand::WrapInList { .. }
        | TypedCommand::UnwrapFromList
        | TypedCommand::IndentListItem
        | TypedCommand::OutdentListItem
        | TypedCommand::ToggleTaskItemChecked
        | TypedCommand::InsertNode { .. }
        | TypedCommand::ResizeImage { .. }) => structure::plan(context, command),
    }
}
