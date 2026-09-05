#[cfg(test)]
use super::observability::{
    history_snapshot_semantic_fallback_forced, HistorySnapshotSemanticFallbackForTest,
    FORCE_HISTORY_DOCUMENT_SNAPSHOT_FALLBACK,
};
use super::selection::{history_selection_to_relative, resolve_selection, resolved_to_legacy};
use super::validation::DocumentValidationCertificate;
use super::DerivedStateCache;
use crate::boundary::ResourceLimits;
use crate::model::{Document, Mark};
use crate::position::PositionMap;
use crate::schema::Schema;
use crate::yrs_engine;
use crate::yrs_engine::canonical::CanonicalArtifact;
use crate::yrs_engine::codec::YrsDocumentCodec;
use crate::yrs_engine::{OperationError, OperationResult, RelativeSelection, ResolvedSelection};
use std::sync::Arc;
use yrs::branch::{Branch, BranchID};
use yrs::types::xml::XmlFragmentRef;
use yrs::ReadTxn;

#[derive(Debug, Clone)]
pub(crate) struct HistoryDocumentSnapshot {
    pub(super) document: Document,
    pub(super) canonical_artifact: CanonicalArtifact,
    pub(super) position_map: PositionMap,
    pub(super) rendered_text: String,
    pub(super) rendered_scalars: u32,
    pub(super) document_text_bytes: usize,
    pub(super) document_node_count: usize,
    pub(super) render_blocks: Arc<crate::render::incremental::CachedRenderBlocks>,
    pub(super) validation_certificate: DocumentValidationCertificate,
    pub(super) resource_limits: ResourceLimits,
    pub(super) editing_limits: yrs_engine::EditingLimits,
    pub(super) max_length: Option<u32>,
    pub(super) schema_fingerprint: Arc<str>,
    pub(super) fragment_name: Arc<str>,
    pub(super) scope: Option<yrs_engine::DocumentScope>,
    pub(super) retained_bytes: usize,
}

/// One exact decoded history-candidate read plus an optional pure admission
/// proof. No CRDT snapshot or mutation-seed publication work occurs while
/// constructing this value.
pub(crate) struct PreparedHistoryCandidateRead {
    pub(super) json: serde_json::Value,
    pub(super) admission: Option<AdmittedHistoryCandidateRead>,
}

impl PreparedHistoryCandidateRead {
    pub(crate) fn into_parts(self) -> (serde_json::Value, Option<AdmittedHistoryCandidateRead>) {
        (self.json, self.admission)
    }
}

/// Non-Clone proof that the sole retained-history codec read matched the
/// retained document and every stable restoration seal. Snapshot allocation
/// is deliberately deferred until semantic fast-path eligibility succeeds.
pub(crate) struct AdmittedHistoryCandidateRead {
    pub(super) request_id: u64,
    pub(super) source_document: Document,
    pub(super) canonical_artifact: CanonicalArtifact,
    pub(super) resource_limits: ResourceLimits,
    pub(super) editing_limits: yrs_engine::EditingLimits,
    pub(super) max_length: Option<u32>,
    pub(super) store_token: usize,
    pub(super) fragment_id: BranchID,
    pub(super) schema_fingerprint: Arc<str>,
    pub(super) yrs_state_epoch: u64,
    pub(super) document_revision: u64,
}

impl AdmittedHistoryCandidateRead {
    pub(super) fn validate_request(&self, request_id: u64) -> OperationResult<()> {
        if self.request_id != request_id {
            return Err(OperationError::engine_invariant_failed(
                request_id,
                None,
                "history candidate read request is stale or contradictory",
            ));
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn validate_restoration<T: ReadTxn>(
        &self,
        request_id: u64,
        txn: &T,
        fragment: &XmlFragmentRef,
        snapshot: &HistoryDocumentSnapshot,
        resource_limits: &ResourceLimits,
        editing_limits: &yrs_engine::EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
        yrs_state_epoch: u64,
        document_revision: u64,
    ) -> OperationResult<()> {
        self.validate_request(request_id)?;
        let matches = self.store_token == txn.store() as *const _ as usize
            && self.fragment_id == AsRef::<Branch>::as_ref(fragment).id()
            && self
                .source_document
                .shares_root_storage_with(&snapshot.document)
            && self.canonical_artifact.ptr_eq(&snapshot.canonical_artifact)
            && self
                .canonical_artifact
                .matches_exact_source_document(&snapshot.document)
            && self.resource_limits == *resource_limits
            && self.editing_limits == *editing_limits
            && self.max_length == max_length
            && self.schema_fingerprint.as_ref() == schema_fingerprint
            && self.yrs_state_epoch == yrs_state_epoch
            && self.document_revision == document_revision;
        if !matches {
            return Err(OperationError::engine_invariant_failed(
                request_id,
                None,
                "history snapshot restoration admission is stale or contradictory",
            ));
        }
        Ok(())
    }

    pub(super) fn mint_capability<T: ReadTxn>(
        self,
        request_id: u64,
        txn: &T,
        fragment: &XmlFragmentRef,
    ) -> OperationResult<HistoryMutationLookupCapability> {
        self.validate_request(request_id)?;
        if self.store_token != txn.store() as *const _ as usize
            || self.fragment_id != AsRef::<Branch>::as_ref(fragment).id()
        {
            return Err(OperationError::engine_invariant_failed(
                request_id,
                None,
                "history candidate read storage is stale or contradictory",
            ));
        }
        let history_store_snapshot =
            yrs_engine::mutation::MutationLookupSeed::prepare_history_store_snapshot(
                request_id,
                txn,
                self.resource_limits.max_encoded_state_bytes,
            )?;
        let proof = AdmittedHistoryMutationLookupProof {
            source_document: self.source_document,
            canonical_artifact: self.canonical_artifact,
            resource_limits: self.resource_limits,
            editing_limits: self.editing_limits,
            max_length: self.max_length,
            store_token: self.store_token,
            fragment_id: self.fragment_id,
            schema_fingerprint: self.schema_fingerprint,
            yrs_state_epoch: self.yrs_state_epoch,
            document_revision: self.document_revision,
            history_store_snapshot,
        };
        Ok(HistoryMutationLookupCapability {
            request_id,
            seed: yrs_engine::mutation::MutationLookupSeed::from_admitted_history_proof(proof),
        })
    }

    #[cfg(test)]
    pub(crate) fn mint_capability_for_test<T: ReadTxn>(
        self,
        request_id: u64,
        txn: &T,
        fragment: &XmlFragmentRef,
    ) -> OperationResult<HistoryMutationLookupCapability> {
        self.mint_capability(request_id, txn, fragment)
    }
}

/// Non-Clone, one-shot ownership of the only mutation lookup seed that may
/// carry retained history store/document evidence.
#[derive(Debug)]
pub(crate) struct HistoryMutationLookupCapability {
    pub(super) request_id: u64,
    pub(super) seed: yrs_engine::mutation::MutationLookupSeed,
}

#[derive(Debug)]
pub(crate) struct RestoredHistoryDocumentState {
    pub(crate) state: DerivedStateCache,
    pub(crate) candidate_publication: HistoryMutationLookupCapability,
}

impl HistoryMutationLookupCapability {
    pub(super) fn validate_request(&self, request_id: u64) -> OperationResult<()> {
        if self.request_id != request_id {
            return Err(OperationError::engine_invariant_failed(
                request_id,
                None,
                "history mutation lookup capability request is stale or contradictory",
            ));
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_candidate_publication<T: ReadTxn>(
        self,
        request_id: u64,
        txn: &T,
        fragment: &XmlFragmentRef,
        schema: &Schema,
        source_document: &Document,
        canonical_artifact: &CanonicalArtifact,
        resource_limits: &ResourceLimits,
        editing_limits: &yrs_engine::EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
        yrs_state_epoch: u64,
        document_revision: u64,
    ) -> OperationResult<Arc<yrs_engine::mutation::MutationLookupSeed>> {
        self.validate_request(request_id)?;
        self.seed.prepare_candidate_publication(
            request_id,
            txn,
            fragment,
            schema,
            source_document,
            canonical_artifact,
            resource_limits,
            editing_limits,
            max_length,
            schema_fingerprint,
            yrs_state_epoch,
            document_revision,
        )
    }

    pub(super) fn prepare_unavailable_placeholder(
        self,
        request_id: u64,
    ) -> OperationResult<(Arc<yrs_engine::mutation::MutationLookupSeed>, Self)> {
        self.validate_request(request_id)?;
        // MutationLookupSeed is Clone for the general lifecycle, but retained
        // history evidence may only be duplicated inside this capability
        // boundary. The clone is immediately consumed by the stripping
        // publication operation; only its proof-free Arc can escape.
        let unavailable = self
            .seed
            .clone()
            .try_publish_history_unavailable(request_id)?;
        Ok((unavailable, self))
    }

    #[cfg(test)]
    pub(crate) fn into_unavailable_seed_for_test(
        self,
        request_id: u64,
    ) -> OperationResult<Arc<yrs_engine::mutation::MutationLookupSeed>> {
        self.validate_request(request_id)?;
        self.seed.try_publish_history_unavailable(request_id)
    }
}

/// Unforgeable handoff from the exact derived-state read factory into the
/// private mutation binding constructor. Fields remain private here.
pub(crate) struct AdmittedHistoryMutationLookupProof {
    pub(super) source_document: Document,
    pub(super) canonical_artifact: CanonicalArtifact,
    pub(super) resource_limits: ResourceLimits,
    pub(super) editing_limits: yrs_engine::EditingLimits,
    pub(super) max_length: Option<u32>,
    pub(super) store_token: usize,
    pub(super) fragment_id: BranchID,
    pub(super) schema_fingerprint: Arc<str>,
    pub(super) yrs_state_epoch: u64,
    pub(super) document_revision: u64,
    pub(super) history_store_snapshot: yrs_engine::mutation::HistoryStoreSnapshotEvidence,
}

impl AdmittedHistoryMutationLookupProof {
    #[allow(clippy::type_complexity)]
    pub(crate) fn into_seed_parts(
        self,
    ) -> (
        Document,
        CanonicalArtifact,
        ResourceLimits,
        yrs_engine::EditingLimits,
        Option<u32>,
        usize,
        BranchID,
        Arc<str>,
        u64,
        u64,
        yrs_engine::mutation::HistoryStoreSnapshotEvidence,
    ) {
        (
            self.source_document,
            self.canonical_artifact,
            self.resource_limits,
            self.editing_limits,
            self.max_length,
            self.store_token,
            self.fragment_id,
            self.schema_fingerprint,
            self.yrs_state_epoch,
            self.document_revision,
            self.history_store_snapshot,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HistoryDocumentSnapshotRetainedBytes(usize);

impl HistoryDocumentSnapshotRetainedBytes {
    pub(crate) fn get(self) -> usize {
        self.0
    }
}

pub(crate) struct HistoryDocumentSnapshotRetainedInput<'a> {
    pub document: &'a Document,
    pub canonical_artifact: &'a CanonicalArtifact,
    pub position_map: &'a PositionMap,
    pub rendered_text: &'a String,
    pub render_blocks: &'a crate::render::incremental::CachedRenderBlocks,
    pub schema_fingerprint: &'a str,
    pub fragment_name: &'a str,
    pub scope: Option<&'a yrs_engine::DocumentScope>,
}

pub(super) fn arc_allocation_bound(payload_bytes: usize) -> Option<usize> {
    // Two strong/weak counters plus one word for allocator padding and
    // alignment conservatively bound the Arc allocation header.
    payload_bytes.checked_add(std::mem::size_of::<[usize; 3]>())
}

pub(crate) fn history_document_snapshot_retained_bytes(
    input: HistoryDocumentSnapshotRetainedInput<'_>,
) -> Option<HistoryDocumentSnapshotRetainedBytes> {
    if !input
        .canonical_artifact
        .matches_exact_source_document(input.document)
    {
        return None;
    }
    let retained_charge = input
        .canonical_artifact
        .history_snapshot_retained_charge()?;
    history_document_snapshot_retained_bytes_with_precomputed_document_charge(
        retained_charge.source_document_retained_bytes,
        retained_charge.canonical_retained_bytes,
        input.position_map,
        input.rendered_text,
        input.render_blocks,
        input.schema_fingerprint,
        input.fragment_name,
        input.scope,
    )
}

#[allow(clippy::too_many_arguments)]
#[allow(dead_code)]
pub(crate) fn history_document_snapshot_retained_bytes_with_canonical_charge(
    document: &Document,
    canonical_retained_bytes: usize,
    position_map: &PositionMap,
    rendered_text: &String,
    render_blocks: &crate::render::incremental::CachedRenderBlocks,
    schema_fingerprint: &str,
    fragment_name: &str,
    scope: Option<&yrs_engine::DocumentScope>,
) -> Option<HistoryDocumentSnapshotRetainedBytes> {
    history_document_snapshot_retained_bytes_with_precomputed_document_charge(
        document.history_snapshot_retained_bytes()?,
        canonical_retained_bytes,
        position_map,
        rendered_text,
        render_blocks,
        schema_fingerprint,
        fragment_name,
        scope,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn history_document_snapshot_retained_bytes_with_precomputed_document_charge(
    document_retained_bytes: usize,
    canonical_retained_bytes: usize,
    position_map: &PositionMap,
    rendered_text: &String,
    render_blocks: &crate::render::incremental::CachedRenderBlocks,
    schema_fingerprint: &str,
    fragment_name: &str,
    scope: Option<&yrs_engine::DocumentScope>,
) -> Option<HistoryDocumentSnapshotRetainedBytes> {
    // These immutable payloads are shallow-cloned into the snapshot and may
    // otherwise become unreachable after the next edit. Each helper walks the
    // complete owned capacity recursively with checked arithmetic. Shared node
    // roots are deliberately overcounted across the three payloads; that keeps
    // admission conservative without allocator-identity bookkeeping.
    let shared_payload_bytes = document_retained_bytes
        .checked_add(canonical_retained_bytes)?
        .checked_add(render_blocks.history_snapshot_retained_bytes()?)?;

    let snapshot_allocation_bytes =
        arc_allocation_bound(std::mem::size_of::<HistoryDocumentSnapshot>())?;
    let position_map_bytes = position_map.history_snapshot_clone_retained_bytes()?;
    let schema_arc_bytes = arc_allocation_bound(schema_fingerprint.len())?;
    let validation_schema_arc_bytes = arc_allocation_bound(schema_fingerprint.len())?;
    let fragment_arc_bytes = arc_allocation_bound(fragment_name.len())?;
    let scope_string_bytes = scope.map_or(Some(0), |scope| {
        scope
            .document_id
            .capacity()
            .checked_add(scope.lineage_id.capacity())
    })?;

    snapshot_allocation_bytes
        .checked_add(position_map_bytes)?
        .checked_add(rendered_text.capacity())?
        .checked_add(schema_arc_bytes)?
        .checked_add(validation_schema_arc_bytes)?
        .checked_add(fragment_arc_bytes)?
        .checked_add(scope_string_bytes)?
        .checked_add(shared_payload_bytes)
        .map(HistoryDocumentSnapshotRetainedBytes)
}

impl HistoryDocumentSnapshot {
    pub(crate) fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn admits_candidate_read(
        &self,
        candidate_json: &serde_json::Value,
        fragment: &XmlFragmentRef,
        schema: &Schema,
        resource_limits: &ResourceLimits,
        editing_limits: &yrs_engine::EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
        fragment_name: &str,
        scope: Option<&yrs_engine::DocumentScope>,
    ) -> bool {
        #[cfg(test)]
        if FORCE_HISTORY_DOCUMENT_SNAPSHOT_FALLBACK.get() {
            return false;
        }
        crate::boundary::json_values_equal_stack_safe(
            self.canonical_artifact.value(),
            candidate_json,
        ) && self
            .canonical_artifact
            .matches_exact_source_document(&self.document)
            && self.canonical_artifact.schema_fingerprint() == schema_fingerprint
            && self.canonical_artifact.format_version()
                == yrs_engine::canonical::CANONICAL_ARTIFACT_FORMAT_VERSION
            && crate::schema::schema_fingerprint(schema) == schema_fingerprint
            && matches!(
                AsRef::<Branch>::as_ref(fragment).id(),
                BranchID::Root(name) if name.as_ref() == fragment_name
            )
            && self.resource_limits == *resource_limits
            && self.editing_limits == *editing_limits
            && self.max_length == max_length
            && self.schema_fingerprint.as_ref() == schema_fingerprint
            && self.fragment_name.as_ref() == fragment_name
            && self.scope.as_ref() == scope
            && self.validation_certificate.resource_limits == *resource_limits
            && self.validation_certificate.schema_fingerprint.as_ref() == schema_fingerprint
            && (self
                .validation_certificate
                .canonical_artifact
                .ptr_eq(&self.canonical_artifact)
                || (self.validation_certificate.canonical_fingerprint()
                    == self.canonical_artifact.sha256()
                    && self.validation_certificate.canonical_serialized_len
                        == self.canonical_artifact.serialized_len()))
            && self.validation_certificate.raw_text_scalars
                == self.canonical_artifact.text_scalar_len()
            && self.validation_certificate.raw_text_utf8_bytes
                == self.canonical_artifact.text_utf8_bytes()
            && self.validation_certificate.stats.node_count == self.document_node_count
            && self.validation_certificate.stats.node_count <= resource_limits.max_document_nodes
            && self.validation_certificate.stats.max_depth <= resource_limits.max_document_depth
            && self.validation_certificate.metrics.metadata_bytes <= resource_limits.max_input_bytes
            && self.validation_certificate.metrics.validation_work
                <= resource_limits.max_document_nodes.saturating_mul(128)
            && self.document_text_bytes == self.validation_certificate.raw_text_utf8_bytes
            && self.canonical_artifact.serialized_len() <= editing_limits.max_derived_output_bytes
            && max_length
                .is_none_or(|limit| self.canonical_artifact.text_scalar_len() <= u64::from(limit))
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_candidate_read<T: ReadTxn>(
        &self,
        request_id: u64,
        txn: &T,
        fragment: &XmlFragmentRef,
        schema: &Schema,
        resource_limits: &ResourceLimits,
        editing_limits: &yrs_engine::EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
        fragment_name: &str,
        scope: Option<&yrs_engine::DocumentScope>,
        yrs_state_epoch: u64,
        document_revision: u64,
    ) -> OperationResult<PreparedHistoryCandidateRead> {
        // This is the sole full fragment projection on the retained-snapshot
        // path. The same value both drives HistoryDocumentSnapshot admission
        // and becomes the generic fallback input.
        let json = YrsDocumentCodec::new(schema, resource_limits)
            .read_json(fragment, txn)
            .map_err(|error| history_candidate_read_error(request_id, error))?;
        if !self.admits_candidate_read(
            &json,
            fragment,
            schema,
            resource_limits,
            editing_limits,
            max_length,
            schema_fingerprint,
            fragment_name,
            scope,
        ) {
            return Ok(PreparedHistoryCandidateRead {
                json,
                admission: None,
            });
        }
        let admission = AdmittedHistoryCandidateRead {
            request_id,
            source_document: self.document.clone(),
            canonical_artifact: self.canonical_artifact.clone(),
            resource_limits: resource_limits.clone(),
            editing_limits: editing_limits.clone(),
            max_length,
            store_token: txn.store() as *const _ as usize,
            fragment_id: AsRef::<Branch>::as_ref(fragment).id(),
            schema_fingerprint: Arc::clone(&self.schema_fingerprint),
            yrs_state_epoch,
            document_revision,
        };
        Ok(PreparedHistoryCandidateRead {
            json,
            admission: Some(admission),
        })
    }
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_history_candidate_read_for_test<T: ReadTxn>(
    request_id: u64,
    txn: &T,
    fragment: &XmlFragmentRef,
    schema: &Schema,
    source_document: &Document,
    canonical_artifact: &CanonicalArtifact,
    resource_limits: &ResourceLimits,
    editing_limits: &yrs_engine::EditingLimits,
    max_length: Option<u32>,
    schema_fingerprint: &str,
    yrs_state_epoch: u64,
    document_revision: u64,
) -> OperationResult<PreparedHistoryCandidateRead> {
    let json = YrsDocumentCodec::new(schema, resource_limits)
        .read_json(fragment, txn)
        .map_err(|error| history_candidate_read_error(request_id, error))?;
    let admission =
        if crate::boundary::json_values_equal_stack_safe(canonical_artifact.value(), &json)
            && canonical_artifact.matches_exact_source_document(source_document)
            && canonical_artifact.schema_fingerprint() == schema_fingerprint
            && canonical_artifact.format_version()
                == yrs_engine::canonical::CANONICAL_ARTIFACT_FORMAT_VERSION
            && crate::schema::schema_fingerprint(schema) == schema_fingerprint
        {
            Some(AdmittedHistoryCandidateRead {
                request_id,
                source_document: source_document.clone(),
                canonical_artifact: canonical_artifact.clone(),
                resource_limits: resource_limits.clone(),
                editing_limits: editing_limits.clone(),
                max_length,
                store_token: txn.store() as *const _ as usize,
                fragment_id: AsRef::<Branch>::as_ref(fragment).id(),
                schema_fingerprint: Arc::from(schema_fingerprint),
                yrs_state_epoch,
                document_revision,
            })
        } else {
            None
        };
    Ok(PreparedHistoryCandidateRead { json, admission })
}

pub(super) fn history_candidate_read_error(
    request_id: u64,
    error: yrs_engine::YrsEngineError,
) -> OperationError {
    if let (Some(limit), Some(actual)) = (error.limit, error.actual) {
        let field = if error.code == "INPUT_LIMIT_EXCEEDED" {
            "maxEncodedStateBytes"
        } else {
            "document"
        };
        OperationError::document_limit_exceeded(
            request_id,
            None,
            field,
            u64::try_from(limit).unwrap_or(u64::MAX),
            u64::try_from(actual).unwrap_or(u64::MAX),
        )
    } else {
        OperationError::engine_invariant_failed(request_id, None, error.message)
    }
}

impl DerivedStateCache {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn capture_history_document_snapshot(
        &self,
        resource_limits: &ResourceLimits,
        editing_limits: &yrs_engine::EditingLimits,
        max_length: Option<u32>,
        fragment_name: &str,
        scope: Option<&yrs_engine::DocumentScope>,
        retained_bytes: HistoryDocumentSnapshotRetainedBytes,
    ) -> Arc<HistoryDocumentSnapshot> {
        Arc::new(HistoryDocumentSnapshot {
            document: self.document.clone(),
            canonical_artifact: self.canonical_artifact.clone(),
            position_map: self.position_map.clone(),
            rendered_text: self.rendered_text.clone(),
            rendered_scalars: self.rendered_scalars,
            document_text_bytes: self.document_text_bytes,
            document_node_count: self.document_node_count,
            render_blocks: Arc::clone(&self.render_blocks),
            validation_certificate: self.validation_certificate.clone(),
            resource_limits: resource_limits.clone(),
            editing_limits: editing_limits.clone(),
            max_length,
            schema_fingerprint: Arc::from(self.schema_fingerprint.as_str()),
            fragment_name: Arc::from(fragment_name),
            scope: scope.cloned(),
            retained_bytes: retained_bytes.get(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn restore_history_document_snapshot<T: ReadTxn>(
        request_id: u64,
        snapshot: &HistoryDocumentSnapshot,
        admission: AdmittedHistoryCandidateRead,
        txn: &T,
        fragment: &XmlFragmentRef,
        schema: &Schema,
        expected_relative_selection: &RelativeSelection,
        expected_resolved_selection: &ResolvedSelection,
        stored_marks: Option<Vec<Mark>>,
        resource_limits: &ResourceLimits,
        editing_limits: &yrs_engine::EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
        document_revision: u64,
        state_revision: u64,
        yrs_state_epoch: u64,
    ) -> OperationResult<Option<RestoredHistoryDocumentState>> {
        admission.validate_restoration(
            request_id,
            txn,
            fragment,
            snapshot,
            resource_limits,
            editing_limits,
            max_length,
            schema_fingerprint,
            yrs_state_epoch,
            document_revision,
        )?;
        if {
            #[cfg(test)]
            {
                history_snapshot_semantic_fallback_forced(
                    HistorySnapshotSemanticFallbackForTest::RenderIdentity,
                )
            }
            #[cfg(not(test))]
            {
                false
            }
        } || snapshot.rendered_scalars < snapshot.position_map.total_scalars()
            || !snapshot
                .render_blocks
                .matches_identity(&snapshot.document, schema_fingerprint)
        {
            return Ok(None);
        }
        #[cfg(test)]
        if history_snapshot_semantic_fallback_forced(
            HistorySnapshotSemanticFallbackForTest::RelativeSelection,
        ) {
            return Ok(None);
        }
        let Some(relative_selection) = history_selection_to_relative(
            txn,
            fragment,
            expected_relative_selection,
            expected_resolved_selection,
            schema,
        ) else {
            return Ok(None);
        };
        #[cfg(test)]
        if history_snapshot_semantic_fallback_forced(
            HistorySnapshotSemanticFallbackForTest::ResolvedSelection,
        ) {
            return Ok(None);
        }
        let Some(resolved_selection) = resolve_selection(
            txn,
            fragment,
            &relative_selection,
            schema,
            &snapshot.document,
            &snapshot.position_map,
            &snapshot.rendered_text,
        ) else {
            return Ok(None);
        };
        #[cfg(test)]
        if history_snapshot_semantic_fallback_forced(
            HistorySnapshotSemanticFallbackForTest::ResolvedMismatch,
        ) {
            return Ok(None);
        }
        if &resolved_selection != expected_resolved_selection {
            return Ok(None);
        }
        let mut validation_certificate = snapshot.validation_certificate.clone();
        validation_certificate.document_revision = document_revision;
        validation_certificate.state_revision = state_revision;
        validation_certificate.yrs_state_epoch = yrs_state_epoch;
        let capability = admission.mint_capability(request_id, txn, fragment)?;
        let (mutation_lookup_seed, candidate_publication) =
            capability.prepare_unavailable_placeholder(request_id)?;
        let state = Self {
            document: snapshot.document.clone(),
            canonical_artifact: snapshot.canonical_artifact.clone(),
            position_map: snapshot.position_map.clone(),
            rendered_text: snapshot.rendered_text.clone(),
            rendered_scalars: snapshot.rendered_scalars,
            document_text_bytes: snapshot.document_text_bytes,
            document_node_count: snapshot.document_node_count,
            legacy_selection: resolved_to_legacy(&resolved_selection),
            relative_selection,
            resolved_selection,
            stored_marks,
            document_revision,
            state_revision,
            schema_fingerprint: schema_fingerprint.into(),
            render_blocks: Arc::clone(&snapshot.render_blocks),
            mutation_lookup_seed,
            validation_certificate,
            localized_text_index: None,
            active_state_certificate: None,
        };
        Ok(Some(RestoredHistoryDocumentState {
            state,
            candidate_publication,
        }))
    }
}
