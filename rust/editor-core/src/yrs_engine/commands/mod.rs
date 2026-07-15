//! Serialization-ready editor commands and pure typed-transaction planning.

mod format;
mod text;

use std::collections::HashMap;

use crate::model::{Document, Mark};
use crate::position::PositionMap;
use crate::schema::Schema;

use super::{
    CommandPlan::NotApplicable, OperationResult, ResolvedSelection, RevisionedPosition,
    RevisionedRange, TypedTransaction,
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

pub(crate) struct PlanningContext<'a> {
    pub request_id: u64,
    pub revision: u64,
    pub document: &'a Document,
    pub position_map: &'a PositionMap,
    pub selection: &'a ResolvedSelection,
    pub stored_marks: Option<&'a [Mark]>,
    pub schema: &'a Schema,
    pub resource_limits: &'a crate::boundary::ResourceLimits,
    pub editing_limits: &'a crate::yrs_engine::EditingLimits,
    pub max_length: Option<u32>,
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
        TypedCommand::ApplyListType { .. }
        | TypedCommand::WrapInList { .. }
        | TypedCommand::UnwrapFromList
        | TypedCommand::IndentListItem
        | TypedCommand::OutdentListItem
        | TypedCommand::ToggleTaskItemChecked
        | TypedCommand::InsertNode { .. }
        | TypedCommand::ResizeImage { .. } => Ok(NotApplicable),
    }
}
