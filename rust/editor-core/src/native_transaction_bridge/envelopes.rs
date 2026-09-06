#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InputRequestEnvelope {
    version: u32,
    #[serde(deserialize_with = "deserialize_canonical_u64")]
    request_id: u64,
    #[serde(deserialize_with = "deserialize_canonical_u64")]
    base_document_revision: u64,
    text: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CommandRequestEnvelope {
    version: u32,
    #[serde(deserialize_with = "deserialize_canonical_u64")]
    request_id: u64,
    #[serde(deserialize_with = "deserialize_canonical_u64")]
    base_document_revision: u64,
    command: CommandEnvelope,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SelectionRequestEnvelope {
    version: u32,
    #[serde(deserialize_with = "deserialize_canonical_u64")]
    request_id: u64,
    #[serde(deserialize_with = "deserialize_canonical_u64")]
    base_document_revision: u64,
    selection: SelectionEnvelope,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LocalApiRequestEnvelope {
    version: u32,
    #[serde(deserialize_with = "deserialize_canonical_u64")]
    request_id: u64,
    #[serde(deserialize_with = "deserialize_canonical_u64")]
    base_document_revision: u64,
    #[serde(default)]
    set_json: Option<serde_json::Value>,
    #[serde(default)]
    set_html: Option<String>,
    history: HistoryModeEnvelope,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum HistoryModeEnvelope {
    UndoableBoundary,
    ResetAndClear,
}

impl From<HistoryModeEnvelope> for ReplacementHistory {
    fn from(mode: HistoryModeEnvelope) -> Self {
        match mode {
            HistoryModeEnvelope::UndoableBoundary => Self::UndoableBoundary,
            HistoryModeEnvelope::ResetAndClear => Self::ResetAndClear,
        }
    }
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PositionEnvelope {
    offset: u32,
    kind: OffsetKindEnvelope,
    #[serde(default)]
    affinity: Option<AffinityEnvelope>,
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
enum OffsetKindEnvelope {
    Scalar,
    Utf16,
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
enum AffinityEnvelope {
    Before,
    After,
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RangeEnvelope {
    from: PositionEnvelope,
    to: PositionEnvelope,
}

impl From<PositionEnvelope> for RevisionedPosition {
    fn from(position: PositionEnvelope) -> Self {
        Self {
            offset: position.offset,
            kind: match position.kind {
                OffsetKindEnvelope::Scalar => EditorOffsetKind::Scalar,
                OffsetKindEnvelope::Utf16 => EditorOffsetKind::Utf16,
            },
            affinity: match position.affinity {
                Some(AffinityEnvelope::Before) => Affinity::Before,
                Some(AffinityEnvelope::After) => Affinity::After,
                None => DEFAULT_POSITION_AFFINITY,
            },
        }
    }
}

impl From<RangeEnvelope> for RevisionedRange {
    fn from(range: RangeEnvelope) -> Self {
        Self {
            from: range.from.into(),
            to: range.to.into(),
        }
    }
}

/// Data-only mirror of the full [`TypedCommand`] surface.
#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
enum CommandEnvelope {
    InsertText {
        text: String,
    },
    DeleteRange {
        range: RangeEnvelope,
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
        #[serde(rename = "markType")]
        mark_type: String,
    },
    SetMark {
        #[serde(rename = "markType")]
        mark_type: String,
        #[serde(default)]
        attrs: HashMap<String, serde_json::Value>,
    },
    UnsetMark {
        #[serde(rename = "markType")]
        mark_type: String,
    },
    ToggleHeading {
        level: u8,
    },
    ToggleCodeBlock,
    ToggleBlockquote,
    ApplyListType {
        #[serde(rename = "listType")]
        list_type: String,
    },
    WrapInList {
        #[serde(rename = "listType")]
        list_type: String,
        #[serde(rename = "itemType")]
        item_type: String,
    },
    UnwrapFromList,
    IndentListItem,
    OutdentListItem,
    ToggleTaskItemChecked,
    InsertNode {
        #[serde(rename = "nodeType")]
        node_type: String,
    },
    UpdateNodeAttrs {
        #[serde(rename = "docPos")]
        doc_pos: u32,
        #[serde(default)]
        attrs: HashMap<String, serde_json::Value>,
    },
    ResizeImage {
        at: PositionEnvelope,
        width: u32,
        height: u32,
    },
    MoveSelection {
        range: RangeEnvelope,
        at: PositionEnvelope,
    },
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NativeIntentRequestEnvelope {
    version: u32,
    #[serde(deserialize_with = "deserialize_canonical_u64")]
    request_id: u64,
    #[serde(deserialize_with = "deserialize_canonical_u64")]
    owner_id: u64,
    #[serde(deserialize_with = "deserialize_canonical_u64")]
    position_epoch: u64,
    intent: NativeIntentEnvelope,
}

#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
enum NativeIntentEnvelope {
    SetSelection {
        anchor: u32,
        head: u32,
    },
    InsertText {
        anchor: u32,
        head: u32,
        text: String,
    },
    ReplaceSelectionText {
        anchor: u32,
        head: u32,
        text: String,
    },
    DeleteBackward {
        anchor: u32,
        head: u32,
    },
    DeleteForward {
        anchor: u32,
        head: u32,
    },
    DeleteSurroundingText {
        anchor: u32,
        head: u32,
        before: u32,
        after: u32,
    },
    DeleteRange {
        anchor: u32,
        head: u32,
    },
    SplitBlock {
        anchor: u32,
        head: u32,
    },
    DeleteAndSplit {
        anchor: u32,
        head: u32,
    },
    InsertContentHtml {
        anchor: u32,
        head: u32,
        html: String,
    },
    InsertContentJson {
        anchor: u32,
        head: u32,
        json: serde_json::Value,
    },
    Command {
        anchor: u32,
        head: u32,
        command: CommandEnvelope,
    },
}

impl From<CommandEnvelope> for TypedCommand {
    fn from(command: CommandEnvelope) -> Self {
        match command {
            CommandEnvelope::InsertText { text } => Self::InsertText { text },
            CommandEnvelope::DeleteRange { range } => Self::DeleteRange {
                range: range.into(),
            },
            CommandEnvelope::DeleteBackward => Self::DeleteBackward,
            CommandEnvelope::ReplaceSelectionText { text } => Self::ReplaceSelectionText { text },
            CommandEnvelope::SplitBlock => Self::SplitBlock,
            CommandEnvelope::DeleteAndSplit => Self::DeleteAndSplit,
            CommandEnvelope::InsertContentJson { json } => Self::InsertContentJson { json },
            CommandEnvelope::InsertContentHtml { html } => Self::InsertContentHtml { html },
            CommandEnvelope::ToggleMark { mark_type } => Self::ToggleMark { mark_type },
            CommandEnvelope::SetMark { mark_type, attrs } => Self::SetMark { mark_type, attrs },
            CommandEnvelope::UnsetMark { mark_type } => Self::UnsetMark { mark_type },
            CommandEnvelope::ToggleHeading { level } => Self::ToggleHeading { level },
            CommandEnvelope::ToggleCodeBlock => Self::ToggleCodeBlock,
            CommandEnvelope::ToggleBlockquote => Self::ToggleBlockquote,
            CommandEnvelope::ApplyListType { list_type } => Self::ApplyListType { list_type },
            CommandEnvelope::WrapInList {
                list_type,
                item_type,
            } => Self::WrapInList {
                list_type,
                item_type,
            },
            CommandEnvelope::UnwrapFromList => Self::UnwrapFromList,
            CommandEnvelope::IndentListItem => Self::IndentListItem,
            CommandEnvelope::OutdentListItem => Self::OutdentListItem,
            CommandEnvelope::ToggleTaskItemChecked => Self::ToggleTaskItemChecked,
            CommandEnvelope::InsertNode { node_type } => Self::InsertNode { node_type },
            CommandEnvelope::UpdateNodeAttrs { doc_pos, attrs } => {
                Self::UpdateNodeAttrs { doc_pos, attrs }
            }
            CommandEnvelope::ResizeImage { at, width, height } => Self::ResizeImage {
                at: at.into(),
                width,
                height,
            },
            CommandEnvelope::MoveSelection { range, at } => Self::MoveSelection {
                range: range.into(),
                at: at.into(),
            },
        }
    }
}

#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum SelectionEnvelope {
    Text {
        anchor: PositionEnvelope,
        head: PositionEnvelope,
    },
    Node {
        at: PositionEnvelope,
    },
    Atom {
        #[serde(rename = "docPos")]
        doc_pos: u32,
        edge: AtomSelectionEdge,
    },
    All,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
enum AtomSelectionEdge {
    Node,
    Before,
    After,
}

impl From<SelectionEnvelope> for SelectionInput {
    fn from(selection: SelectionEnvelope) -> Self {
        match selection {
            SelectionEnvelope::Text { anchor, head } => Self::Text {
                anchor: anchor.into(),
                head: head.into(),
            },
            SelectionEnvelope::Node { at } => Self::Node { at: at.into() },
            SelectionEnvelope::All => Self::All,
            SelectionEnvelope::Atom { .. } => {
                unreachable!("atom selections require document mapping")
            }
        }
    }
}
