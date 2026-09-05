mod candidate;
mod candidate_cache;
mod commands;
mod commit;
mod commit_installation;
mod compilation;
mod history_state;
mod imports;
mod mutation_context;
mod outbound;
mod remote;
mod selection_commit;
mod snapshots;
#[cfg(test)]
mod test_hooks;
mod transaction_result;
mod transactions;
mod undo_redo;

use super::canonical::CanonicalSchemaContext;
#[cfg(test)]
use super::compiler::CompiledTransaction;
use super::derived_state::DerivedStateCache;
#[cfg(test)]
use super::YrsDocumentCodec;
use super::{
    DocumentScope, DocumentSnapshot, EditingLimits, TransactionOrigin, YrsEngineError,
    YrsEngineResult,
};
use crate::boundary::ResourceLimits;
use crate::model::Document;
use crate::position::PositionMap;
use crate::schema::Schema;
use crate::serialize::to_html;
use candidate::{
    admit_candidate_derived_output, build_await_remote_candidate,
    build_derived_state_for_candidate, build_local_empty_candidate,
};
#[cfg(test)]
use candidate::{CandidateDocument, EngineDocumentState};
use candidate_cache::{encode_state_bounded, PreparedCandidateCache};
#[cfg(test)]
use candidate_cache::{
    equivalent_private_candidate_doc, fresh_utf16_doc_excluding, fresh_utf16_doc_excluding_with,
    prepare_import_candidate_cache, retained_import_state_charge, seal_candidate_state_vector,
    utf16_doc,
};
#[cfg(test)]
use history_state::history_metadata_bytes;
pub(crate) use imports::admit_local_import_document;
#[cfg(test)]
use imports::ValidatedImportDocument;
#[cfg(test)]
use outbound::OutboundUpdateSink;
#[cfg(test)]
use remote::admit_max_encoded_state_len;
pub use remote::PreparedRemoteUpdate;
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
#[cfg(test)]
use test_hooks::{
    check_compiled_commit_preparation_stage_for_test, mark_compiled_commit_durable_write_for_test,
    reset_encoded_state_reuse_counts_for_test, reset_import_receipt_sha256_counts_for_test,
    reset_import_receipt_state_decodings_for_test, reset_import_state_encoding_counts_for_test,
    reset_prepared_candidate_cache_counts_for_test, set_compiled_commit_stage_failpoint_for_test,
    set_outbound_staging_copy_failure_for_test,
    set_quarantined_update_reservation_failure_for_test,
    take_compiled_commit_authority_counts_for_test, take_encoded_state_reuse_counts_for_test,
    take_import_receipt_sha256_counts_for_test, take_import_receipt_state_decodings_for_test,
    take_import_state_encoding_counts_for_test, take_prepared_candidate_cache_counts_for_test,
    CompiledCommitPreparationStage,
};
use yrs::sync::time::{Clock, SystemClock};
use yrs::Doc;
use yrs::{ReadTxn, Transact};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitializationMode {
    LocalEmpty,
    AwaitRemote,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineRenderState {
    Loading,
    Ready,
}

#[derive(Debug, Clone)]
pub struct YrsEngineConfig {
    pub schema: Schema,
    pub fragment_name: String,
    pub initialization_mode: InitializationMode,
    pub resource_limits: ResourceLimits,
    pub editing_limits: EditingLimits,
    pub max_length: Option<u32>,
    pub scope: Option<DocumentScope>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EngineCommit {
    pub changed: bool,
    pub revision: u64,
}

pub struct YrsDocumentEngine {
    doc: Doc,
    fragment_name: String,
    schema: Schema,
    resource_limits: ResourceLimits,
    editing_limits: EditingLimits,
    max_length: Option<u32>,
    scope: Option<DocumentScope>,
    schema_fingerprint: String,
    canonical_schema: CanonicalSchemaContext,
    derived_state: Option<DerivedStateCache>,
    revision: u64,
    state_revision: u64,
    yrs_state_epoch: u64,
    last_committed_origin: Option<TransactionOrigin>,
    document_origin: super::DocumentOrigin,
    durable_client_ids: HashSet<u64>,
    /// Dependency-pending standard updates are quarantined outside the live
    /// authoritative Doc until their complete merged state can be validated.
    quarantined_remote_update: Option<Vec<u8>>,
    /// Invalidates every outstanding [`PreparedRemoteUpdate`] seal on engine
    /// transitions that do NOT change revision/state-revision/epoch or the
    /// store handle: (a) a new dependency-pending payload entering quarantine
    /// (committing an older prepare would silently discard it), and (b) the
    /// unchanged fast paths of snapshot restore and canonical-equal imports,
    /// which clear the quarantine and rebind the bounded history replay chain
    /// (committing across that rebind could both resurrect intentionally
    /// discarded dependency bytes and violate the prepared replay-slot
    /// capacity invariants mid-install). Every other quarantine or history
    /// transition also changes a revision/epoch, which the seal covers.
    remote_seal_generation: u64,
    /// The engine-owned awareness codec: the sole `yrs::sync::Awareness`
    /// bound to the authoritative `Doc`, rebound on every store swap.
    awareness: Option<super::awareness::AwarenessCodec>,
    history: super::history::YrsHistory,
    /// An exact private replica used only to prove the next local commit. It is
    /// never exposed as editor authority and is consumed on use, so any
    /// recoverable preparation failure automatically drops it rather than
    /// publishing partially prepared state.
    prepared_candidate_cache: Option<PreparedCandidateCache>,
}

impl YrsDocumentEngine {
    pub fn new(config: YrsEngineConfig) -> YrsEngineResult<Self> {
        Self::new_with_history_clock(config, Arc::new(SystemClock))
    }

    pub fn new_with_snapshot(
        config: YrsEngineConfig,
        snapshot: &DocumentSnapshot,
    ) -> YrsEngineResult<Self> {
        if config.initialization_mode != InitializationMode::AwaitRemote {
            return Err(YrsEngineError::new(
                "CONFIG_INVALID",
                "snapshot initialization is only valid for an awaiting room document",
            )
            .with_details(json!({ "field": "initializationMode" })));
        }
        let mut engine = Self::new(config)?;
        engine.restore_snapshot(snapshot)?;
        Ok(engine)
    }

    pub fn new_with_history_clock(
        config: YrsEngineConfig,
        history_clock: Arc<dyn Clock>,
    ) -> YrsEngineResult<Self> {
        let YrsEngineConfig {
            schema,
            fragment_name,
            initialization_mode,
            resource_limits,
            editing_limits,
            max_length,
            scope,
        } = config;
        resource_limits.validate()?;
        editing_limits.validate()?;
        validate_config_metadata(&fragment_name, scope.as_ref(), &resource_limits)?;
        let canonical_schema = CanonicalSchemaContext::new(&schema);
        let schema_fingerprint = canonical_schema.schema_fingerprint().to_owned();
        let candidate = match initialization_mode {
            InitializationMode::LocalEmpty => build_local_empty_candidate(
                &schema,
                &canonical_schema,
                &fragment_name,
                &resource_limits,
            )?,
            InitializationMode::AwaitRemote => {
                build_await_remote_candidate(&fragment_name, &resource_limits)?
            }
        };
        admit_candidate_derived_output(&candidate, &editing_limits)?;
        let derived_state = build_derived_state_for_candidate(
            &candidate,
            &schema,
            &resource_limits,
            &editing_limits,
            max_length,
            &schema_fingerprint,
            &fragment_name,
            &canonical_schema,
            0,
            None,
            0,
            0,
            0,
        )?;
        let history_fragment = {
            let txn = candidate.doc.transact();
            txn.get_xml_fragment(fragment_name.as_str())
                .ok_or_else(|| {
                    YrsEngineError::new(
                        "CODEC_INVARIANT_FAILED",
                        "initialized Yrs fragment is missing while binding history",
                    )
                })?
        };
        let history = super::history::YrsHistory::new(
            &candidate.doc,
            &history_fragment,
            editing_limits.clone(),
            resource_limits.max_encoded_state_bytes,
            history_clock,
        );

        Ok(Self {
            doc: candidate.doc,
            fragment_name,
            schema,
            resource_limits,
            editing_limits,
            max_length,
            scope,
            schema_fingerprint,
            canonical_schema,
            derived_state,
            revision: 0,
            state_revision: 0,
            yrs_state_epoch: 0,
            last_committed_origin: None,
            document_origin: super::DocumentOrigin::Import,
            durable_client_ids: candidate.durable_client_ids,
            quarantined_remote_update: None,
            remote_seal_generation: 0,
            awareness: None,
            history,
            prepared_candidate_cache: None,
        })
    }

    pub fn is_ready(&self) -> bool {
        self.derived_state.is_some()
    }

    pub fn render_state(&self) -> EngineRenderState {
        if self.is_ready() {
            EngineRenderState::Ready
        } else {
            EngineRenderState::Loading
        }
    }

    #[cfg(test)]
    fn prepared_candidate_cache_store_token_for_test(&self) -> Option<usize> {
        self.prepared_candidate_cache
            .as_ref()
            .map(PreparedCandidateCache::store_token)
    }

    /// Production surface: the engine-owned awareness codec, lazily bound to the
    /// authoritative `Doc`. The codec never exposes the document, a
    /// transaction, or the raw `Awareness` handle.
    pub fn awareness(&mut self) -> &mut super::awareness::AwarenessCodec {
        let doc = &self.doc;
        self.awareness
            .get_or_insert_with(|| super::awareness::AwarenessCodec::bind(doc))
    }

    /// Task 10 wiring: read-only resolution of one peer awareness sticky
    /// cursor point (the serialized `StickyIndex` form the sticky-position
    /// surface produces) to a ProseMirror document position against the
    /// current authoritative store. Invalid or unresolvable points return
    /// `None` — the runtime degrades the peer projection to cursor-less
    /// rather than erroring. Never mutates document state.
    pub fn resolve_awareness_sticky_doc_pos(&self, sticky_json: &serde_json::Value) -> Option<u32> {
        let sticky: yrs::StickyIndex = serde_json::from_value(sticky_json.clone()).ok()?;
        let txn = self.doc.transact();
        let fragment = txn.get_xml_fragment(self.fragment_name.as_str())?;
        super::position::sticky_index_to_doc_pos(&txn, &fragment, &sticky, &self.schema)
    }

    /// Sealed awareness surface: materialize two valid document positions as
    /// sticky Yrs indices in this engine's current document context. Callers
    /// receive only the wire JSON; neither the document nor its transaction
    /// crosses the engine boundary.
    pub(crate) fn awareness_sticky_cursor(
        &self,
        anchor: u32,
        head: u32,
    ) -> Option<serde_json::Value> {
        let txn = self.doc.transact();
        let fragment = txn.get_xml_fragment(self.fragment_name.as_str())?;
        let collapsed = anchor == head;
        let anchor = super::cursor_sticky_index_from_doc_pos(
            &txn,
            &fragment,
            anchor,
            collapsed,
            &self.schema,
        )?;
        let head = super::cursor_sticky_index_from_doc_pos(
            &txn,
            &fragment,
            head,
            collapsed,
            &self.schema,
        )?;
        Some(serde_json::json!({ "anchor": anchor, "head": head }))
    }

    pub fn document(&self) -> Option<&Document> {
        self.debug_assert_derived_revision_keys();
        let state = self.derived_state.as_ref()?;
        Some(&state.document)
    }

    pub(crate) fn cached_render_blocks(
        &self,
    ) -> Option<Arc<crate::render::incremental::CachedRenderBlocks>> {
        self.debug_assert_derived_revision_keys();
        self.derived_state
            .as_ref()
            .map(|state| Arc::clone(&state.render_blocks))
    }

    pub(crate) fn block_atom_ids(&self) -> Option<HashMap<u32, String>> {
        let txn = self.doc.transact();
        let fragment = txn.get_xml_fragment(self.fragment_name.as_str())?;
        super::position::block_atom_ids(&txn, &fragment, &self.schema)
    }

    pub fn document_json(&self) -> Option<serde_json::Value> {
        self.debug_assert_derived_revision_keys();
        self.derived_state.as_ref().map(|state| {
            crate::boundary::clone_json_value_stack_safe(state.canonical_artifact.value())
        })
    }

    pub(crate) fn document_json_string(&self) -> Option<String> {
        self.debug_assert_derived_revision_keys();
        self.derived_state.as_ref().map(|state| {
            String::from_utf8(crate::boundary::serialize_json_value_stack_safe(
                state.canonical_artifact.value(),
                state.canonical_artifact.serialized_len(),
            ))
            .expect("serialized JSON is UTF-8")
        })
    }

    pub fn document_html(&self) -> Option<String> {
        self.document()
            .map(|document| to_html(document, &self.schema))
    }

    #[allow(dead_code)]
    pub fn encoded_state(&self) -> YrsEngineResult<Vec<u8>> {
        encode_state_bounded(&self.doc, &self.resource_limits)
    }

    #[allow(dead_code)]
    pub fn has_document_state(&self) -> bool {
        !self.doc.transact().state_vector().is_empty()
    }

    pub fn revision(&self) -> u64 {
        self.debug_assert_derived_revision_keys();
        self.revision
    }

    pub fn state_revision(&self) -> u64 {
        self.debug_assert_derived_revision_keys();
        self.state_revision
    }

    /// Production audit surface: the Yrs state epoch, so full before/after
    /// session audits can pin epoch stability across atomic rejections.
    #[allow(dead_code)]
    pub fn yrs_state_epoch(&self) -> u64 {
        self.yrs_state_epoch
    }

    pub fn position_map(&self) -> Option<&PositionMap> {
        self.debug_assert_derived_revision_keys();
        self.derived_state.as_ref().map(|state| &state.position_map)
    }

    pub(crate) fn build_position_epoch_boundaries(
        &self,
    ) -> Option<Vec<crate::position_epoch::BoundaryAnchors>> {
        self.debug_assert_derived_revision_keys();
        let state = self.derived_state.as_ref()?;
        let txn = self.doc.transact();
        let fragment = txn.get_xml_fragment(self.fragment_name.as_str())?;
        let count = usize::try_from(state.position_map.total_scalars())
            .ok()?
            .checked_add(1)?;
        let mut boundaries = Vec::new();
        boundaries.try_reserve_exact(count).ok()?;
        let mut previous: Option<(u32, crate::position_epoch::BoundaryAnchors)> = None;
        for scalar_offset in 0..=state.position_map.total_scalars() {
            let doc_pos = state
                .position_map
                .scalar_to_doc(scalar_offset, &state.document);
            let anchors = if let Some((previous_doc_pos, previous_anchors)) = &previous {
                if *previous_doc_pos == doc_pos {
                    previous_anchors.clone()
                } else {
                    super::position::boundary_anchors_from_doc_pos(
                        &txn,
                        &fragment,
                        doc_pos,
                        &self.schema,
                    )?
                }
            } else {
                super::position::boundary_anchors_from_doc_pos(
                    &txn,
                    &fragment,
                    doc_pos,
                    &self.schema,
                )?
            };
            previous = Some((doc_pos, anchors.clone()));
            boundaries.push(anchors);
        }
        Some(boundaries)
    }

    pub(crate) fn resolve_position_epoch_boundary(
        &self,
        boundary: &crate::position_epoch::BoundaryAnchors,
        affinity: super::Affinity,
        original_offset: u32,
    ) -> Option<(u32, bool)> {
        self.debug_assert_derived_revision_keys();
        let state = self.derived_state.as_ref()?;
        let txn = self.doc.transact();
        let fragment = txn.get_xml_fragment(self.fragment_name.as_str())?;
        let (leaf, ancestors, opposite_leaf, opposite_ancestors) = match affinity {
            super::Affinity::Before => (
                &boundary.before,
                &boundary.ancestor_before,
                &boundary.after,
                &boundary.ancestor_after,
            ),
            super::Affinity::After => (
                &boundary.after,
                &boundary.ancestor_after,
                &boundary.before,
                &boundary.ancestor_before,
            ),
        };
        for (fallback, sticky) in std::iter::once((false, leaf))
            .chain(ancestors.iter().map(|sticky| (true, sticky)))
            .chain(std::iter::once((true, opposite_leaf)))
            .chain(opposite_ancestors.iter().map(|sticky| (true, sticky)))
        {
            if let Some(doc_pos) =
                super::position::sticky_index_to_doc_pos(&txn, &fragment, sticky, &self.schema)
            {
                return Some((
                    state.position_map.doc_to_scalar(doc_pos, &state.document),
                    fallback,
                ));
            }
        }
        Some((
            original_offset.min(state.position_map.total_scalars()),
            true,
        ))
    }

    pub fn relative_selection(&self) -> Option<&super::RelativeSelection> {
        self.debug_assert_derived_revision_keys();
        self.derived_state
            .as_ref()
            .map(|state| &state.relative_selection)
    }

    #[allow(dead_code)]
    pub fn resolved_selection(&self) -> Option<&super::ResolvedSelection> {
        self.debug_assert_derived_revision_keys();
        self.derived_state
            .as_ref()
            .map(|state| &state.resolved_selection)
    }

    #[allow(dead_code)]
    pub fn stored_marks(&self) -> Option<&[crate::model::Mark]> {
        self.debug_assert_derived_revision_keys();
        self.derived_state
            .as_ref()
            .and_then(|state| state.stored_marks.as_deref())
    }

    pub fn client_id(&self) -> u64 {
        self.doc.client_id().get()
    }

    #[allow(dead_code)]
    pub fn fragment_name(&self) -> &str {
        &self.fragment_name
    }

    #[allow(dead_code)]
    pub fn schema_fingerprint(&self) -> &str {
        &self.schema_fingerprint
    }

    #[allow(dead_code)]
    pub fn scope(&self) -> Option<&DocumentScope> {
        self.scope.as_ref()
    }

    #[allow(dead_code)]
    pub fn last_committed_origin(&self) -> Option<TransactionOrigin> {
        self.last_committed_origin
    }

    pub fn document_origin(&self) -> super::DocumentOrigin {
        self.document_origin
    }

    pub(crate) fn mark_document_origin_native_view(&mut self) {
        self.document_origin = super::DocumentOrigin::NativeView;
    }

    pub fn resource_limits(&self) -> &ResourceLimits {
        &self.resource_limits
    }

    #[allow(dead_code)]
    pub fn editing_limits(&self) -> &EditingLimits {
        &self.editing_limits
    }

    #[allow(dead_code)]
    pub fn max_length(&self) -> Option<u32> {
        self.max_length
    }

    fn debug_assert_derived_revision_keys(&self) {
        if let Some(state) = &self.derived_state {
            debug_assert_eq!(state.document_revision, self.revision);
            debug_assert_eq!(state.state_revision, self.state_revision);
            debug_assert!(state
                .render_blocks
                .matches_identity(&state.document, &state.schema_fingerprint));
            debug_assert_eq!(
                state.document_node_count,
                crate::editor_state::document_node_count(state.document.root())
            );
        }
    }

    fn next_revision(&self) -> YrsEngineResult<u64> {
        self.revision.checked_add(1).ok_or_else(|| {
            YrsEngineError::new(
                "REVISION_OVERFLOW",
                "document revision cannot be incremented",
            )
            .with_details(json!({ "field": "revision" }))
        })
    }

    fn reset_history_binding(&mut self) {
        let fragment = {
            let txn = self.doc.transact();
            txn.get_xml_fragment(self.fragment_name.as_str())
                .expect("ready Yrs document retains the history fragment")
        };
        self.history.rebind(&self.doc, &fragment);
        // Rebinding rebuilds the bounded replay chain (and, on the unchanged
        // restore/import fast paths, accompanies a quarantine clear) without
        // any revision/epoch change. Invalidate every outstanding prepared
        // remote update so a later commit can neither resurrect discarded
        // dependency bytes nor install against the reset replay chain.
        self.remote_seal_generation = self.remote_seal_generation.wrapping_add(1);
    }

    fn next_durable_revisions(&self) -> YrsEngineResult<(u64, u64, u64)> {
        let document_revision = self.next_revision()?;
        let state_revision = self.state_revision.checked_add(1).ok_or_else(|| {
            YrsEngineError::new("REVISION_OVERFLOW", "state revision cannot be incremented")
                .with_details(json!({ "field": "stateRevision" }))
        })?;
        let yrs_state_epoch = self.yrs_state_epoch.checked_add(1).ok_or_else(|| {
            YrsEngineError::new("REVISION_OVERFLOW", "Yrs state epoch cannot be incremented")
                .with_details(json!({ "field": "yrsStateEpoch" }))
        })?;
        Ok((document_revision, state_revision, yrs_state_epoch))
    }
}

fn checked_operation_increment(
    request_id: u64,
    value: u64,
    field: &'static str,
) -> super::OperationResult<u64> {
    value
        .checked_add(1)
        .ok_or_else(|| super::OperationError::revision_overflow(request_id, field))
}

fn merge_operation_details(mapped: &mut super::OperationError, source: Option<serde_json::Value>) {
    let Some(serde_json::Value::Object(source)) = source else {
        return;
    };
    let target = mapped
        .details
        .get_or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    let serde_json::Value::Object(target) = target else {
        return;
    };
    for (key, value) in source {
        if key != "field" {
            target.insert(key, value);
        }
    }
}

fn validate_config_metadata(
    fragment_name: &str,
    scope: Option<&DocumentScope>,
    limits: &ResourceLimits,
) -> YrsEngineResult<()> {
    let fields = [
        ("fragmentName", fragment_name.len()),
        (
            "documentId",
            scope.map(|scope| scope.document_id.len()).unwrap_or(0),
        ),
        (
            "lineageId",
            scope.map(|scope| scope.lineage_id.len()).unwrap_or(0),
        ),
    ];
    for (field, actual) in fields {
        if actual > limits.max_input_bytes {
            return Err(YrsEngineError::limit(
                "INPUT_LIMIT_EXCEEDED",
                limits.max_input_bytes,
                actual,
            )
            .with_details(json!({ "field": field })));
        }
    }
    let total = fields
        .into_iter()
        .fold(0usize, |total, (_, bytes)| total.saturating_add(bytes));
    if total > limits.max_input_bytes {
        return Err(
            YrsEngineError::limit("INPUT_LIMIT_EXCEEDED", limits.max_input_bytes, total)
                .with_details(json!({ "field": "metadata" })),
        );
    }
    Ok(())
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;
