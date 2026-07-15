mod codec;
#[allow(dead_code)] // Task 7 installs compiled transactions into the live Yrs engine.
mod compiler;
mod derived_state;
mod editing_limits;
mod engine;
mod error;
mod history;
mod mutation;
#[cfg(test)]
#[path = "mutation_tests.rs"]
mod mutation_tests;
mod operation;
mod origin;
mod position;
mod snapshot;
mod update_preflight;

const RAW_STORAGE_WORK_MULTIPLIER: usize = 128;

fn raw_storage_work_limit(limits: &crate::boundary::ResourceLimits) -> usize {
    limits
        .max_document_nodes
        .saturating_mul(RAW_STORAGE_WORK_MULTIPLIER)
}

pub(crate) use codec::YrsDocumentCodec;
pub use editing_limits::{
    EditingLimitOverrides, EditingLimits, HARD_MAX_DERIVED_OUTPUT_BYTES,
    HARD_MAX_OPERATIONS_PER_TRANSACTION, HARD_MAX_UNDO_GROUPS, HARD_MAX_UNDO_RETAINED_UNITS,
};
pub use engine::{EngineCommit, InitializationMode, YrsDocumentEngine, YrsEngineConfig};
pub use error::{YrsEngineError, YrsEngineResult};
pub use operation::{
    Affinity, EditorOffsetKind, HistoryPolicy, OperationError, OperationResult, RenderUpdate,
    ResolvedPoint, ResolvedSelection, RevisionedPosition, RevisionedRange, SelectionInput,
    SelectionIntent, TransactionCommit, TypedOperation, TypedTransaction, TypedTransactionResult,
};
pub use origin::TransactionOrigin;
#[cfg(test)]
pub(crate) use position::doc_pos_to_sticky_index;
pub(crate) use position::editor_offset_to_doc_pos;
pub(crate) use position::{cursor_sticky_index_from_doc_pos, sticky_index_to_doc_pos};
pub use position::{
    doc_pos_to_relative_point, relative_point_to_doc_pos, relative_selection_to_selection,
    revisioned_position_to_relative_point, scalar_offset_to_utf16, utf16_offset_to_scalar,
    RelativePoint, RelativeSelection,
};

pub use snapshot::{DocumentScope, DocumentSnapshot, SNAPSHOT_FORMAT_VERSION};
