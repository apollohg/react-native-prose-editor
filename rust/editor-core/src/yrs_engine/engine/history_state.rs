use crate::boundary::ResourceLimits;
use crate::yrs_engine;
use crate::yrs_engine::canonical::CanonicalArtifact;
use crate::yrs_engine::derived_state::DerivedStateCache;
use crate::yrs_engine::{DocumentScope, YrsEngineError};

pub(super) fn history_local_state(
    state: &DerivedStateCache,
    fragment_name: &str,
    scope: Option<&DocumentScope>,
    resource_limits: &ResourceLimits,
    editing_limits: &yrs_engine::EditingLimits,
    max_length: Option<u32>,
    document_snapshot_retained_bytes: Option<
        yrs_engine::derived_state::HistoryDocumentSnapshotRetainedBytes,
    >,
) -> yrs_engine::history::HistoryLocalState {
    let document_snapshot = document_snapshot_retained_bytes.map(|retained_bytes| {
        state.capture_history_document_snapshot(
            resource_limits,
            editing_limits,
            max_length,
            fragment_name,
            scope,
            retained_bytes,
        )
    });
    yrs_engine::history::HistoryLocalState {
        relative_selection: state.relative_selection.clone(),
        resolved_selection: state.resolved_selection.clone(),
        stored_marks: state.stored_marks.clone(),
        text_length: state.canonical_artifact.text_scalar_len(),
        canonical_fingerprint: state.canonical_artifact.sha256(),
        derived_output_bytes: state.canonical_artifact.serialized_len(),
        metadata_bytes: history_metadata_bytes(state.stored_marks.as_deref(), fragment_name)
            .saturating_add(
                document_snapshot
                    .as_deref()
                    .map(yrs_engine::derived_state::HistoryDocumentSnapshot::retained_bytes)
                    .unwrap_or(0),
            ),
        document_snapshot,
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct HistoryDocumentSnapshotRetainedPair {
    pub(super) before: yrs_engine::derived_state::HistoryDocumentSnapshotRetainedBytes,
    pub(super) after: yrs_engine::derived_state::HistoryDocumentSnapshotRetainedBytes,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn history_document_snapshots_fit(
    before: &DerivedStateCache,
    after_document: &crate::model::Document,
    after_canonical_artifact: &CanonicalArtifact,
    after_derivations: &yrs_engine::compiler::CompiledDocumentDerivations,
    after_render_blocks: &crate::render::incremental::CachedRenderBlocks,
    after_stored_marks: Option<&[crate::model::Mark]>,
    schema_fingerprint: &str,
    fragment_name: &str,
    scope: Option<&DocumentScope>,
    metadata_limit: usize,
) -> Option<HistoryDocumentSnapshotRetainedPair> {
    history_document_snapshots_fit_with_canonical_charge(
        before,
        after_document,
        after_canonical_artifact.history_snapshot_retained_bytes()?,
        after_derivations,
        after_render_blocks,
        after_stored_marks,
        schema_fingerprint,
        fragment_name,
        scope,
        metadata_limit,
    )
}

#[allow(clippy::too_many_arguments)]
fn history_document_snapshots_fit_with_canonical_charge(
    before: &DerivedStateCache,
    after_document: &crate::model::Document,
    after_canonical_retained_bytes: usize,
    after_derivations: &yrs_engine::compiler::CompiledDocumentDerivations,
    after_render_blocks: &crate::render::incremental::CachedRenderBlocks,
    after_stored_marks: Option<&[crate::model::Mark]>,
    schema_fingerprint: &str,
    fragment_name: &str,
    scope: Option<&DocumentScope>,
    metadata_limit: usize,
) -> Option<HistoryDocumentSnapshotRetainedPair> {
    let after_document_retained_bytes = after_document.history_snapshot_retained_bytes()?;
    history_document_snapshots_fit_with_precomputed_after_charge(
        before,
        after_canonical_retained_bytes,
        after_document_retained_bytes,
        after_derivations,
        after_render_blocks,
        after_stored_marks,
        schema_fingerprint,
        fragment_name,
        scope,
        metadata_limit,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn history_document_snapshots_fit_with_precomputed_after_charge(
    before: &DerivedStateCache,
    after_canonical_retained_bytes: usize,
    after_document_retained_bytes: usize,
    after_derivations: &yrs_engine::compiler::CompiledDocumentDerivations,
    after_render_blocks: &crate::render::incremental::CachedRenderBlocks,
    after_stored_marks: Option<&[crate::model::Mark]>,
    schema_fingerprint: &str,
    fragment_name: &str,
    scope: Option<&DocumentScope>,
    metadata_limit: usize,
) -> Option<HistoryDocumentSnapshotRetainedPair> {
    let before_retained = yrs_engine::derived_state::history_document_snapshot_retained_bytes(
        yrs_engine::derived_state::HistoryDocumentSnapshotRetainedInput {
            document: &before.document,
            canonical_artifact: &before.canonical_artifact,
            position_map: &before.position_map,
            rendered_text: &before.rendered_text,
            render_blocks: &before.render_blocks,
            schema_fingerprint,
            fragment_name,
            scope,
        },
    )?;
    let after_retained =
        yrs_engine::derived_state::history_document_snapshot_retained_bytes_with_precomputed_document_charge(
            after_document_retained_bytes,
            after_canonical_retained_bytes,
            &after_derivations.position_map,
            &after_derivations.rendered_text,
            after_render_blocks,
            schema_fingerprint,
            fragment_name,
            scope,
        )?;
    let total = history_metadata_bytes(before.stored_marks.as_deref(), fragment_name)
        .checked_add(history_metadata_bytes(after_stored_marks, fragment_name))
        .and_then(|bytes| bytes.checked_add(before_retained.get()))
        .and_then(|bytes| bytes.checked_add(after_retained.get()))?;
    (total <= metadata_limit).then_some(HistoryDocumentSnapshotRetainedPair {
        before: before_retained,
        after: after_retained,
    })
}

pub(super) fn history_snapshot_template(
    canonical_artifact: &CanonicalArtifact,
    stored_marks: Option<&[crate::model::Mark]>,
    fragment_name: &str,
    document_snapshot_retained_bytes: Option<
        yrs_engine::derived_state::HistoryDocumentSnapshotRetainedBytes,
    >,
) -> yrs_engine::history::HistorySnapshotTemplate {
    history_snapshot_template_from_identity(
        canonical_artifact.text_scalar_len(),
        canonical_artifact.sha256(),
        canonical_artifact.serialized_len(),
        stored_marks,
        fragment_name,
        document_snapshot_retained_bytes,
    )
}

pub(super) fn history_snapshot_template_from_identity(
    text_length: u64,
    canonical_fingerprint: [u8; 32],
    derived_output_bytes: usize,
    stored_marks: Option<&[crate::model::Mark]>,
    fragment_name: &str,
    document_snapshot_retained_bytes: Option<
        yrs_engine::derived_state::HistoryDocumentSnapshotRetainedBytes,
    >,
) -> yrs_engine::history::HistorySnapshotTemplate {
    let retained_bytes = document_snapshot_retained_bytes
        .map(yrs_engine::derived_state::HistoryDocumentSnapshotRetainedBytes::get)
        .unwrap_or(0);
    yrs_engine::history::HistorySnapshotTemplate {
        stored_marks: stored_marks.map(<[crate::model::Mark]>::to_vec),
        text_length,
        canonical_fingerprint,
        derived_output_bytes,
        metadata_bytes: history_metadata_bytes(stored_marks, fragment_name)
            .saturating_add(retained_bytes),
        document_snapshot_retained_bytes,
    }
}

pub(super) fn history_metadata_bytes(
    stored_marks: Option<&[crate::model::Mark]>,
    fragment_name: &str,
) -> usize {
    // Fixed selection metadata covers the bounded relative/resolved selection
    // representation. Stored marks are cloned deeply, so charge their source
    // container and recursive capacities rather than serialized logical size.
    const FIXED_SELECTION_BYTES: usize = 512;
    const EMPTY_MARKS_SEQUENCE_BYTES: usize = 2;
    let stored_marks = stored_marks.unwrap_or_default();
    let mark_bytes = stored_marks
        .len()
        .checked_mul(std::mem::size_of::<crate::model::Mark>())
        .and_then(|slots| {
            stored_marks.iter().try_fold(slots, |total, mark| {
                total.checked_add(mark.history_snapshot_clone_retained_bytes()?)
            })
        })
        .unwrap_or(usize::MAX);
    FIXED_SELECTION_BYTES
        .checked_add(fragment_name.len())
        .and_then(|bytes| bytes.checked_add(EMPTY_MARKS_SEQUENCE_BYTES))
        .and_then(|bytes| bytes.checked_add(mark_bytes))
        .unwrap_or(usize::MAX)
}

pub(super) fn history_operation_error(
    request_id: u64,
    error: YrsEngineError,
) -> yrs_engine::OperationError {
    if let (Some(limit), Some(actual)) = (error.limit, error.actual) {
        let field = if error.code == "INPUT_LIMIT_EXCEEDED" {
            "maxEncodedStateBytes"
        } else {
            "document"
        };
        yrs_engine::OperationError::document_limit_exceeded(
            request_id,
            None,
            field,
            u64::try_from(limit).unwrap_or(u64::MAX),
            u64::try_from(actual).unwrap_or(u64::MAX),
        )
    } else {
        yrs_engine::OperationError::engine_invariant_failed(request_id, None, error.message)
    }
}
