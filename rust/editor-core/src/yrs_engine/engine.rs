use serde_json::json;
use sha2::Digest;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use yrs::branch::{Branch, BranchID};
use yrs::sync::time::{Clock, SystemClock};
use yrs::types::xml::XmlFragmentRef;
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::Update;
use yrs::{
    Assoc, ClientID, Doc, IndexedSequence, OffsetKind, Options, ReadTxn, StateVector, Transact,
    Transaction, Uuid, WriteTxn,
};

use crate::boundary::{
    document_json_container_depth_limit, parse_json_value_stack_safe,
    with_document_stack_for_json_container_depth, BoundedInput, InputKind, ResourceLimits,
};
use crate::model::Document;
use crate::position::update::UpdateMode;
use crate::position::PositionMap;
use crate::schema::{NodeRole, Schema};
use crate::selection::Selection;
use crate::serialize::{
    from_html_with_limits, from_prosemirror_json_with_limits, rehydrate_reserved_html_opaque,
    to_html, to_prosemirror_json, FromHtmlOptions, JsonParseError, ParseError, UnknownTypeMode,
};
use crate::transform::{
    canonicalize_yrs_document, canonicalize_yrs_document_with_evidence,
    validate_importable_marks_with_evidence, CanonicalMarksEvidence, DocumentValidationReport,
    DocumentValidator, StepMap,
};

use super::canonical::{CanonicalArtifact, CanonicalSchemaContext};
use super::compiler::{
    compile_prepared_transaction_with_yrs_and_stored_marks,
    compile_transaction_with_yrs_and_stored_marks, map_position, selectable_void_at,
    CompilationContext, CompiledTransaction, EngineCompilationView, MutationLookupTransition,
    PreparedSemanticAdmission, PreparedSemanticContext,
};
use super::compiler::{
    RelativeSelectionPlan, SelectionPlan, StoredMarksCompilationContext, StoredMarksPlan,
};
use super::derived_state::{
    exact_point_is_representable, history_selection_to_relative, operation_result_to_relative,
    stored_marks_after_selection_change, DerivedStateCache, FinalizedSelectionState,
    ValidatedCandidateContext, ValidatedDocumentEvidence,
};
use super::mutation::{
    execute_mutation_plan, preflight_mutation_plan, YrsMutationAction, YrsMutationPlan,
};
use super::update_preflight::preflight_update_v1;
use super::{
    DocumentScope, DocumentSnapshot, EditingLimits, TransactionOrigin, YrsDocumentCodec,
    YrsEngineError, YrsEngineResult, SNAPSHOT_FORMAT_VERSION,
};

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompiledCommitPreparationStage {
    AllocationProbe,
    OperationPreparation,
    DocumentValidation,
    LookupTransition,
    HistoryReservation,
    HistoryUpdateEncoding,
    SelectionFinalization,
    DerivedStateBuild,
    HistorySnapshotConstruction,
}

#[cfg(test)]
impl CompiledCommitPreparationStage {
    const fn field_name(self) -> &'static str {
        match self {
            Self::AllocationProbe => "allocationProbe",
            Self::OperationPreparation => "operationPreparation",
            Self::DocumentValidation => "documentValidation",
            Self::LookupTransition => "lookupTransition",
            Self::HistoryReservation => "historyReservation",
            Self::HistoryUpdateEncoding => "historyUpdateEncoding",
            Self::SelectionFinalization => "selectionFinalization",
            Self::DerivedStateBuild => "derivedStateBuild",
            Self::HistorySnapshotConstruction => "historySnapshotConstruction",
        }
    }
}

#[cfg(test)]
std::thread_local! {
    static COMPILED_COMMIT_STAGE_FAILPOINT: std::cell::Cell<Option<CompiledCommitPreparationStage>> = const { std::cell::Cell::new(None) };
    static COMPILED_COMMIT_DURABLE_WRITE_OPENED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static COMPILED_COMMIT_AUTHORITY_VALIDATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static COMPILED_COMMIT_LIVE_VIEWS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static PREPARED_CANDIDATE_CACHE_HITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static PREPARED_CANDIDATE_FULL_BOOTSTRAPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static CANDIDATE_BOUNDED_STATE_ENCODINGS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static IMPORT_CANDIDATE_STATE_ENCODINGS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static IMPORT_RECEIPT_STATE_DECODINGS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static IMPORT_RECEIPT_SHA256_MINTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static IMPORT_RECEIPT_SHA256_MATCHES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static COMMIT_CURRENT_STATE_ENCODINGS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static COMMIT_SEALED_STATE_REUSES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static FAIL_QUARANTINED_UPDATE_RESERVATION: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static FAIL_OUTBOUND_STAGING_COPY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
fn set_quarantined_update_reservation_failure_for_test(enabled: bool) {
    FAIL_QUARANTINED_UPDATE_RESERVATION.set(enabled);
}

#[cfg(test)]
fn set_outbound_staging_copy_failure_for_test(enabled: bool) {
    FAIL_OUTBOUND_STAGING_COPY.set(enabled);
}

#[cfg(test)]
fn reset_prepared_candidate_cache_counts_for_test() {
    PREPARED_CANDIDATE_CACHE_HITS.set(0);
    PREPARED_CANDIDATE_FULL_BOOTSTRAPS.set(0);
}

#[cfg(test)]
fn take_prepared_candidate_cache_counts_for_test() -> (usize, usize) {
    let hits = PREPARED_CANDIDATE_CACHE_HITS.replace(0);
    let bootstraps = PREPARED_CANDIDATE_FULL_BOOTSTRAPS.replace(0);
    (hits, bootstraps)
}

#[cfg(test)]
fn reset_encoded_state_reuse_counts_for_test() {
    IMPORT_CANDIDATE_STATE_ENCODINGS.set(0);
    COMMIT_CURRENT_STATE_ENCODINGS.set(0);
    COMMIT_SEALED_STATE_REUSES.set(0);
}

#[cfg(test)]
fn reset_import_state_encoding_counts_for_test() {
    CANDIDATE_BOUNDED_STATE_ENCODINGS.set(0);
    IMPORT_CANDIDATE_STATE_ENCODINGS.set(0);
}

#[cfg(test)]
fn take_import_state_encoding_counts_for_test() -> (usize, usize) {
    (
        CANDIDATE_BOUNDED_STATE_ENCODINGS.replace(0),
        IMPORT_CANDIDATE_STATE_ENCODINGS.replace(0),
    )
}

#[cfg(test)]
fn reset_import_receipt_state_decodings_for_test() {
    IMPORT_RECEIPT_STATE_DECODINGS.set(0);
}

#[cfg(test)]
fn take_import_receipt_state_decodings_for_test() -> usize {
    IMPORT_RECEIPT_STATE_DECODINGS.replace(0)
}

#[cfg(test)]
fn reset_import_receipt_sha256_counts_for_test() {
    IMPORT_RECEIPT_SHA256_MINTS.set(0);
    IMPORT_RECEIPT_SHA256_MATCHES.set(0);
}

#[cfg(test)]
fn take_import_receipt_sha256_counts_for_test() -> (usize, usize) {
    (
        IMPORT_RECEIPT_SHA256_MINTS.replace(0),
        IMPORT_RECEIPT_SHA256_MATCHES.replace(0),
    )
}

#[cfg(test)]
fn take_encoded_state_reuse_counts_for_test() -> (usize, usize, usize) {
    (
        IMPORT_CANDIDATE_STATE_ENCODINGS.replace(0),
        COMMIT_CURRENT_STATE_ENCODINGS.replace(0),
        COMMIT_SEALED_STATE_REUSES.replace(0),
    )
}

#[cfg(test)]
fn set_compiled_commit_stage_failpoint_for_test(stage: Option<CompiledCommitPreparationStage>) {
    COMPILED_COMMIT_STAGE_FAILPOINT.set(stage);
    COMPILED_COMMIT_DURABLE_WRITE_OPENED.set(false);
}

#[cfg(test)]
fn begin_compiled_commit_preparation_for_test() {
    COMPILED_COMMIT_DURABLE_WRITE_OPENED.set(false);
    COMPILED_COMMIT_AUTHORITY_VALIDATIONS.set(0);
    COMPILED_COMMIT_LIVE_VIEWS.set(0);
}

#[cfg(test)]
fn record_compiled_commit_authority_validation_for_test() {
    COMPILED_COMMIT_AUTHORITY_VALIDATIONS.set(
        COMPILED_COMMIT_AUTHORITY_VALIDATIONS
            .get()
            .saturating_add(1),
    );
}

#[cfg(test)]
fn record_compiled_commit_live_view_for_test() {
    COMPILED_COMMIT_LIVE_VIEWS.set(COMPILED_COMMIT_LIVE_VIEWS.get().saturating_add(1));
}

#[cfg(test)]
fn take_compiled_commit_authority_counts_for_test() -> (usize, usize) {
    (
        COMPILED_COMMIT_AUTHORITY_VALIDATIONS.replace(0),
        COMPILED_COMMIT_LIVE_VIEWS.replace(0),
    )
}

#[cfg(test)]
fn mark_compiled_commit_durable_write_for_test() {
    COMPILED_COMMIT_DURABLE_WRITE_OPENED.set(true);
}

#[cfg(test)]
fn check_compiled_commit_preparation_stage_for_test(
    request_id: u64,
    stage: CompiledCommitPreparationStage,
) -> super::OperationResult<()> {
    let durable_write_opened = COMPILED_COMMIT_DURABLE_WRITE_OPENED.get();
    if durable_write_opened || COMPILED_COMMIT_STAGE_FAILPOINT.get() == Some(stage) {
        let phase = if durable_write_opened {
            "postwrite"
        } else {
            "prewrite"
        };
        return Err(super::OperationError::engine_invariant_failed(
            request_id,
            None,
            format!(
                "compiled commit {} preparation failpoint ran {phase}",
                stage.field_name()
            ),
        ));
    }
    Ok(())
}

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

fn selection_requires_fallback_proof<T: ReadTxn>(
    plan: &YrsMutationPlan,
    txn: &T,
    fragment: &XmlFragmentRef,
    selection: &super::RelativeSelection,
) -> bool {
    match selection {
        super::RelativeSelection::Text { anchor, head } => {
            plan.removes_sticky_branch(txn, fragment, &anchor.sticky)
                || plan.removes_sticky_branch(txn, fragment, &head.sticky)
        }
        super::RelativeSelection::Node { point } => {
            plan.removes_sticky_branch(txn, fragment, &point.sticky)
        }
        super::RelativeSelection::All => false,
    }
}

struct FallbackProofContext<'a, Current, Proof> {
    plan: &'a YrsMutationPlan,
    current_txn: &'a Current,
    current_fragment: &'a XmlFragmentRef,
    proof_txn: &'a Proof,
    proof_fragment: &'a XmlFragmentRef,
    schema: &'a Schema,
}

fn required_fallbacks_are_representable<Current: ReadTxn, Proof: ReadTxn>(
    context: FallbackProofContext<'_, Current, Proof>,
    selection: &Selection,
    relative: &super::RelativeSelection,
) -> bool {
    let FallbackProofContext {
        plan,
        current_txn,
        current_fragment,
        proof_txn,
        proof_fragment,
        schema,
    } = context;
    let point_is_valid = |position, point: &super::RelativePoint| {
        !plan.removes_sticky_branch(current_txn, current_fragment, &point.sticky)
            || exact_point_is_representable(proof_txn, proof_fragment, position, point, schema)
    };
    match (selection, relative) {
        (
            Selection::Text { anchor, head },
            super::RelativeSelection::Text {
                anchor: relative_anchor,
                head: relative_head,
            },
        ) => point_is_valid(*anchor, relative_anchor) && point_is_valid(*head, relative_head),
        (Selection::Node { pos }, super::RelativeSelection::Node { point }) => {
            point_is_valid(*pos, point)
        }
        (Selection::All, super::RelativeSelection::All) => true,
        _ => false,
    }
}

enum EngineDocumentState {
    AwaitingRemote,
    Ready {
        document: Document,
        canonical_artifact: CanonicalArtifact,
    },
}

struct CandidateDocument {
    doc: Doc,
    state: EngineDocumentState,
    durable_client_ids: HashSet<u64>,
    validated_import: Option<RootBoundValidationReport>,
    import_acceleration_eligible: bool,
    import_encoded_state_receipt: Option<ImportEncodedStateReceipt>,
}

/// A one-owner capability proving that these exact standard update-v1 bytes
/// were produced from the validated import candidate after its codec
/// round-trip and encoded-state admission completed.
struct ImportEncodedStateReceipt {
    encoded_state: Vec<u8>,
    encoded_state_sha256: [u8; 32],
    state_vector: StateVector,
    fragment_id: BranchID,
    client_id: ClientID,
    guid: Uuid,
    offset_kind: OffsetKind,
    skip_gc: bool,
    delete_set_is_empty: bool,
    lookup_materialization: Option<ImportLookupMaterializationReceipt>,
    lookup_state_verified: bool,
}

struct ImportLookupMaterializationReceipt {
    materialization: super::mutation::ImportLookupMaterialization,
    source_document: Document,
    canonical_artifact: CanonicalArtifact,
    resource_limits: ResourceLimits,
    editing_limits: EditingLimits,
    max_length: Option<u32>,
    schema_token: usize,
    store_token: usize,
}

struct FinalizedImportLookupMaterialization {
    materialization: super::mutation::ImportLookupMaterialization,
    source_document: Document,
    canonical_artifact: CanonicalArtifact,
    resource_limits: ResourceLimits,
    editing_limits: EditingLimits,
    max_length: Option<u32>,
    document_revision: u64,
    yrs_state_epoch: u64,
}

impl ImportEncodedStateReceipt {
    #[allow(clippy::too_many_arguments)]
    fn mint(
        source: &Doc,
        fragment_name: &str,
        encoded_state: Vec<u8>,
        delete_set_is_empty: bool,
        lookup_materialization: Option<super::mutation::ImportLookupMaterialization>,
        source_document: &Document,
        canonical_artifact: &CanonicalArtifact,
        resource_limits: &ResourceLimits,
        editing_limits: &EditingLimits,
        max_length: Option<u32>,
        schema: &Schema,
    ) -> Option<Self> {
        let txn = source.transact();
        if txn.has_missing_updates() {
            return None;
        }
        let state_vector = txn.state_vector();
        let fragment = txn.get_xml_fragment(fragment_name)?;
        let fragment_id = AsRef::<Branch>::as_ref(&fragment).id();
        #[cfg(test)]
        IMPORT_RECEIPT_SHA256_MINTS.set(IMPORT_RECEIPT_SHA256_MINTS.get().saturating_add(1));
        let encoded_state_sha256 = sha2::Sha256::digest(&encoded_state).into();
        let lookup_materialization =
            lookup_materialization.map(|materialization| ImportLookupMaterializationReceipt {
                materialization,
                source_document: source_document.clone(),
                canonical_artifact: canonical_artifact.clone(),
                resource_limits: resource_limits.clone(),
                editing_limits: editing_limits.clone(),
                max_length,
                schema_token: schema as *const _ as usize,
                store_token: txn.store() as *const _ as usize,
            });
        Some(Self {
            encoded_state,
            encoded_state_sha256,
            state_vector,
            fragment_id,
            client_id: source.client_id(),
            guid: source.guid(),
            offset_kind: source.offset_kind(),
            skip_gc: source.skip_gc(),
            delete_set_is_empty,
            lookup_materialization,
            lookup_state_verified: false,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn take_matching_lookup_materialization(
        &mut self,
        source: &Doc,
        fragment_name: &str,
        source_document: &Document,
        canonical_artifact: &CanonicalArtifact,
        resource_limits: &ResourceLimits,
        editing_limits: &EditingLimits,
        max_length: Option<u32>,
        schema: &Schema,
        schema_fingerprint: &str,
        document_revision: u64,
        yrs_state_epoch: u64,
    ) -> Option<FinalizedImportLookupMaterialization> {
        let txn = source.transact();
        let fragment = txn.get_xml_fragment(fragment_name)?;
        let receipt = self.lookup_materialization.take()?;
        #[cfg(test)]
        IMPORT_RECEIPT_SHA256_MATCHES.set(IMPORT_RECEIPT_SHA256_MATCHES.get().saturating_add(1));
        let encoded_state_sha256: [u8; 32] = sha2::Sha256::digest(&self.encoded_state).into();
        let exact = !txn.has_missing_updates()
            && txn.state_vector() == self.state_vector
            && AsRef::<Branch>::as_ref(&fragment).id() == self.fragment_id
            && txn.store() as *const _ as usize == receipt.store_token
            && source.client_id() == self.client_id
            && source.guid() == self.guid
            && source.offset_kind() == self.offset_kind
            && source.skip_gc() == self.skip_gc
            && encoded_state_sha256 == self.encoded_state_sha256
            && self.delete_set_is_empty
            && receipt
                .source_document
                .shares_root_storage_with(source_document)
            && receipt.canonical_artifact.ptr_eq(canonical_artifact)
            && canonical_artifact.matches_exact_source_document(source_document)
            && receipt.resource_limits == *resource_limits
            && receipt.editing_limits == *editing_limits
            && receipt.max_length == max_length
            // The engine-owned Schema is construction-frozen: it has no
            // mutation or replacement API. Its exact address plus the exact
            // canonical artifact/source and that artifact's precomputed
            // fingerprint therefore seal the current schema without hashing
            // its full projection again on this import-only path.
            && receipt.schema_token == schema as *const _ as usize
            && receipt.canonical_artifact.schema_fingerprint() == schema_fingerprint;
        if exact {
            self.lookup_state_verified = true;
        }
        exact.then_some(FinalizedImportLookupMaterialization {
            materialization: receipt.materialization,
            source_document: receipt.source_document,
            canonical_artifact: receipt.canonical_artifact,
            resource_limits: receipt.resource_limits,
            editing_limits: receipt.editing_limits,
            max_length: receipt.max_length,
            document_revision,
            yrs_state_epoch,
        })
    }

    fn into_matching(
        self,
        source: &Doc,
        fragment_name: &str,
        max_encoded_state_bytes: usize,
    ) -> Option<(Vec<u8>, StateVector, BranchID, bool)> {
        if self.encoded_state.len() > max_encoded_state_bytes {
            return None;
        }
        let txn = source.transact();
        let fragment = txn.get_xml_fragment(fragment_name)?;
        let fragment_id = AsRef::<Branch>::as_ref(&fragment).id();
        let metadata_matches = !txn.has_missing_updates()
            && txn.state_vector() == self.state_vector
            && fragment_id == self.fragment_id
            && source.client_id() == self.client_id
            && source.guid() == self.guid
            && source.offset_kind() == self.offset_kind
            && source.skip_gc() == self.skip_gc;
        if !metadata_matches {
            return None;
        }
        let encoded_hash_matches = self.lookup_state_verified || {
            #[cfg(test)]
            IMPORT_RECEIPT_SHA256_MATCHES
                .set(IMPORT_RECEIPT_SHA256_MATCHES.get().saturating_add(1));
            let encoded_state_sha256: [u8; 32] = sha2::Sha256::digest(&self.encoded_state).into();
            encoded_state_sha256 == self.encoded_state_sha256
        };
        encoded_hash_matches.then_some((
            self.encoded_state,
            self.state_vector,
            self.fragment_id,
            self.delete_set_is_empty,
        ))
    }
}

#[derive(Clone)]
struct RootBoundValidationReport {
    source_root: crate::model::Node,
    report: DocumentValidationReport,
}

struct ValidatedImportDocument {
    document: Document,
    canonical_artifact: CanonicalArtifact,
    validation: RootBoundValidationReport,
    carry_import_encoded_state_receipt: bool,
}

impl ValidatedImportDocument {
    fn new(
        document: Document,
        schema: &Schema,
        canonical_schema: &CanonicalSchemaContext,
        resource_limits: &ResourceLimits,
        json_input_len: Option<usize>,
    ) -> YrsEngineResult<Self> {
        let carry_import_encoded_state_receipt = true;
        if contains_reserved_public_json_forge(document.root()) {
            return Err(candidate_invariant_parse_error(
                "public JSON cannot construct reserved opaque HTML metadata",
                "candidate codec round-trip changed the document",
            ));
        }
        let canonical_marks = validate_yrs_mark_representation(&document, schema)?;
        let validation = validate_import_document_report(&document, schema, resource_limits)?;
        let canonical_document =
            canonicalize_yrs_document_with_evidence(&document, schema, canonical_marks);
        let (document, validation) = if canonical_document == document {
            (document, validation)
        } else {
            let validation =
                validate_import_document_report(&canonical_document, schema, resource_limits)?;
            (canonical_document, validation)
        };
        let validation = RootBoundValidationReport {
            source_root: document.root().clone(),
            report: validation,
        };
        let canonical_artifact = if let Some(input_len) = json_input_len {
            canonical_schema.derive_validated_json(
                &document,
                input_len,
                validation.report.metrics.validation_work,
            )
        } else {
            canonical_schema.derive(&document)
        }
        .map_err(|error| {
            candidate_invariant_parse_error(error, "candidate serialization failed")
        })?;
        Ok(Self {
            document,
            canonical_artifact,
            validation,
            carry_import_encoded_state_receipt,
        })
    }
}

/// Session-free import admission for consumers that need the exact local
/// import contract without allocating a Yjs document or editor runtime.
pub(crate) fn admit_local_import_document(
    document: Document,
    schema: &Schema,
    resource_limits: &ResourceLimits,
    editing_limits: &EditingLimits,
    json_input_len: Option<usize>,
) -> YrsEngineResult<Document> {
    let canonical_schema = CanonicalSchemaContext::new(schema);
    let admitted = ValidatedImportDocument::new(
        document,
        schema,
        &canonical_schema,
        resource_limits,
        json_input_len,
    )?;
    admit_canonical_output(&admitted.canonical_artifact, editing_limits)?;
    Ok(admitted.document)
}

fn contains_reserved_public_json_forge(root: &crate::model::Node) -> bool {
    let mut pending = vec![root];
    while let Some(node) = pending.pop() {
        if node.node_type() == "__opaque_json"
            && node
                .attrs()
                .get("original_type")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|node_type| {
                    matches!(node_type, "__opaque" | "__opaque_json" | "__skip")
                })
        {
            return true;
        }
        if let Some(content) = node.content() {
            pending.extend(content.iter());
        }
    }
    false
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

struct PreparedCandidateCache {
    doc: Doc,
    state_vector: StateVector,
    staged_lookup_seed: Option<Arc<super::mutation::MutationLookupSeed>>,
    document_revision: u64,
    yrs_state_epoch: u64,
    encoded_state_seal: Option<EncodedStateSeal>,
}

struct EncodedStateSeal {
    encoded_state: Vec<u8>,
    fragment_id: BranchID,
    client_id: ClientID,
    guid: Uuid,
    offset_kind: OffsetKind,
    skip_gc: bool,
    document_revision: u64,
    yrs_state_epoch: u64,
}

impl PreparedCandidateCache {
    fn take_matching_encoded_state(
        &mut self,
        live_doc: &Doc,
        live_fragment: &XmlFragmentRef,
        mutation_plan: &YrsMutationPlan,
        document_revision: u64,
        yrs_state_epoch: u64,
        max_encoded_state_bytes: usize,
    ) -> Option<Vec<u8>> {
        // A seal is one-shot even when it has gone stale. Keeping rejected
        // bytes would retain an untrusted optimization across another commit.
        let seal = self.encoded_state_seal.take()?;
        let within_current_ceiling =
            retained_import_state_charge(seal.encoded_state.len(), seal.encoded_state.capacity())
                .is_some_and(|retained| retained <= max_encoded_state_bytes);
        let live_fragment_id = AsRef::<Branch>::as_ref(live_fragment).id();
        let candidate_txn = self.doc.transact();
        let candidate_fragment_id = seal.fragment_id.get_branch(&candidate_txn)?.id();
        let matches = within_current_ceiling
            && self.document_revision == document_revision
            && self.yrs_state_epoch == yrs_state_epoch
            && seal.document_revision == document_revision
            && seal.yrs_state_epoch == yrs_state_epoch
            && seal.client_id == live_doc.client_id()
            && seal.guid == live_doc.guid()
            && seal.offset_kind == live_doc.offset_kind()
            && seal.skip_gc == live_doc.skip_gc()
            && seal.client_id == self.doc.client_id()
            && seal.guid == self.doc.guid()
            && seal.offset_kind == self.doc.offset_kind()
            && seal.skip_gc == self.doc.skip_gc()
            && seal.fragment_id == live_fragment_id
            && seal.fragment_id == candidate_fragment_id
            && mutation_plan.matches_sealed_import_state(&self.state_vector);
        matches.then_some(seal.encoded_state)
    }

    fn into_matching_doc(
        self,
        document_revision: u64,
        yrs_state_epoch: u64,
    ) -> Option<(Doc, StateVector)> {
        if self.document_revision != document_revision || self.yrs_state_epoch != yrs_state_epoch {
            return None;
        }
        Some((self.doc, self.state_vector))
    }

    #[cfg(test)]
    fn store_token(&self) -> usize {
        let txn = self.doc.transact();
        txn.store() as *const _ as usize
    }
}

struct PreparedRemoteSeal {
    request_id: u64,
    sealed_doc: Doc,
    admitted_revision: u64,
    admitted_state_revision: u64,
    admitted_epoch: u64,
    sealed_generation: u64,
}

enum PreparedDocumentOutcome {
    Unchanged,
    Changed(Box<PreparedRemoteInstall>),
}

enum PreparedDependencyState {
    Clear,
    Retain(Vec<u8>),
}

/// A one-shot remote Update-v1 admission, sealed to the exact store identity,
/// document revision, state revision, epoch, and quarantine generation that
/// admitted it. Everything fallible (decode preflight, dependency
/// classification, candidate admission, ceilings, and derived-state
/// preparation) already happened during preparation; committing atomically
/// installs the document and dependency candidates. Deliberately non-`Clone`
/// and consumed by value.
pub struct PreparedRemoteUpdate {
    seal: PreparedRemoteSeal,
    document: PreparedDocumentOutcome,
    dependencies: PreparedDependencyState,
}

impl PreparedRemoteUpdate {
    pub fn retained_dependency_bytes(&self) -> usize {
        match &self.dependencies {
            PreparedDependencyState::Clear => 0,
            PreparedDependencyState::Retain(bytes) => bytes.len(),
        }
    }

    pub fn has_pending_dependencies(&self) -> bool {
        matches!(self.dependencies, PreparedDependencyState::Retain(_))
    }
}

/// The proven installation payload for a changed remote update. Every field
/// was validated against the live store during preparation.
struct PreparedRemoteInstall {
    live_update: Update,
    accepted_update: Vec<u8>,
    history_admission: super::history::PreparedExcludedHistoryAdmission,
    next_state: DerivedStateCache,
    prepared_live_seed: Arc<super::mutation::MutationLookupSeed>,
    durable_client_ids: HashSet<u64>,
    next_revision: u64,
    next_state_revision: u64,
    next_epoch: u64,
}

fn retained_import_state_charge(encoded_len: usize, encoded_capacity: usize) -> Option<usize> {
    encoded_len.checked_mul(2)?.checked_add(encoded_capacity)
}

fn seal_candidate_state_vector(
    request_id: u64,
    base: &StateVector,
    actual: StateVector,
    local_client: ClientID,
    admitted_authored_clock_bound: u32,
) -> super::OperationResult<StateVector> {
    let base_local_clock = base.get(&local_client);
    let actual_local_clock = actual.get(&local_client);
    let Some(actual_local_delta) = actual_local_clock.checked_sub(base_local_clock) else {
        return Err(super::OperationError::engine_invariant_failed(
            request_id,
            None,
            "prepared commit candidate regressed its local authored clock",
        ));
    };
    if actual_local_delta > admitted_authored_clock_bound {
        return Err(super::OperationError::engine_invariant_failed(
            request_id,
            None,
            "prepared commit candidate exceeded its admitted authored clock bound",
        ));
    }
    let mut expected = base.clone();
    expected.inc_by(local_client, actual_local_delta);
    if actual != expected {
        return Err(super::OperationError::engine_invariant_failed(
            request_id,
            None,
            "prepared commit candidate changed a nonlocal authored clock",
        ));
    }
    Ok(actual)
}

enum PreparedCompiledHistory {
    Recorded(super::history::PreparedRecordedHistoryAdmission),
    Excluded(super::history::PreparedExcludedHistoryAdmission),
}

enum CompiledCommitDerivedAuthority<'a> {
    Staged(super::prepared_admission::StagedDerivedStateAuthority<'a>),
    Installed(super::prepared_admission::InstalledDerivedStateAuthority<'a>),
}

struct CompiledCommitAuthority<'a, 'doc> {
    derived: CompiledCommitDerivedAuthority<'a>,
    txn: &'a Transaction<'doc>,
    fragment: &'a XmlFragmentRef,
    state_vector: std::cell::OnceCell<StateVector>,
}

impl CompiledCommitAuthority<'_, '_> {
    fn derived(&self) -> &dyn super::prepared_admission::DerivedStateAuthority {
        match &self.derived {
            CompiledCommitDerivedAuthority::Staged(authority) => authority,
            CompiledCommitDerivedAuthority::Installed(authority) => authority,
        }
    }

    fn txn(&self) -> &Transaction<'_> {
        self.txn
    }

    fn fragment(&self) -> &XmlFragmentRef {
        self.fragment
    }

    fn state_vector(&self) -> &StateVector {
        self.state_vector.get_or_init(|| self.txn.state_vector())
    }
}

struct PreparedCompiledCommit {
    request_id: u64,
    origin: TransactionOrigin,
    history_policy: super::HistoryPolicy,
    history: Option<PreparedCompiledHistory>,
    mutation_plan: Option<YrsMutationPlan>,
    history_update: Vec<u8>,
    history_after: Option<super::history::HistoryLocalState>,
    next_derived_state: Option<DerivedStateCache>,
    next_durable_client_ids: HashSet<u64>,
    next_document_revision: u64,
    next_state_revision: u64,
    next_yrs_state_epoch: u64,
    publish_active_state_install: bool,
    publish_active_state_drop: bool,
    result: Option<super::TypedTransactionResult>,
    next_candidate_cache: Option<PreparedCandidateCache>,
}

struct PreparedHistoryCandidateState {
    state: DerivedStateCache,
    encoded_state: Vec<u8>,
    candidate_publication: Option<super::derived_state::HistoryMutationLookupCapability>,
}

struct PreparedHistoryPop {
    request_id: u64,
    candidate_doc: Doc,
    candidate_history: super::history::YrsHistory,
    candidate_state: DerivedStateCache,
    candidate_publication: Option<super::derived_state::HistoryMutationLookupCapability>,
    next_document_revision: u64,
    next_state_revision: u64,
    next_yrs_state_epoch: u64,
    result: Option<super::TypedTransactionResult>,
}

/// Outbound Update-v1 capture seam for one durable local commit.
///
/// A detached sink is a free no-op, so shipped default-feature paths keep
/// byte-identical behavior and cost by construction. An attached sink
/// (the production collaboration runtime) reserves bounded outbox count/bytes/queue
/// storage from the compiler's conservative `outbound_update_upper_bound`
/// and stages a copy of the captured Update-v1 strictly BEFORE the
/// irreversible Yrs write; after the commit installs, the append is
/// infallible. Dropping the sink without committing releases the
/// reservation, keeping rejected operations atomic.
pub(crate) struct OutboundUpdateSink<'a> {
    target: Option<OutboundSinkTarget<'a>>,
}

struct OutboundSinkTarget<'a> {
    outbox: &'a mut crate::collaboration_runtime::CollaborationOutbox,
    staged: Option<(
        crate::collaboration_runtime::outbox::OutboxReservation,
        Vec<u8>,
    )>,
}

impl<'a> OutboundUpdateSink<'a> {
    pub(crate) fn detached() -> Self {
        Self { target: None }
    }

    pub(crate) fn attached(
        outbox: &'a mut crate::collaboration_runtime::CollaborationOutbox,
    ) -> Self {
        Self {
            target: Some(OutboundSinkTarget {
                outbox,
                staged: None,
            }),
        }
    }

    /// Sink over an optionally attached collaboration outbox: sessions
    /// without a runtime edit through a detached (no-op) sink.
    pub(crate) fn from_optional_outbox(
        outbox: Option<&'a mut crate::collaboration_runtime::CollaborationOutbox>,
    ) -> Self {
        match outbox {
            Some(outbox) => Self::attached(outbox),
            None => Self::detached(),
        }
    }

    /// True when a collaboration outbox is attached; callers skip
    /// capture-only encoding work when detached.
    pub(crate) fn is_attached(&self) -> bool {
        self.target.is_some()
    }

    /// Fallible pre-write step: admit outbox count/bytes/storage from the
    /// conservative bound and stage a bounded copy of the captured update.
    pub(crate) fn reserve_and_stage(
        &mut self,
        request_id: u64,
        upper_bound_bytes: usize,
        update_v1: &[u8],
    ) -> super::OperationResult<()> {
        if let Some(target) = self.target.as_mut() {
            debug_assert!(
                target.staged.is_none(),
                "one durable commit stages at most one outbound update",
            );
            let reservation = target
                .outbox
                .reserve_document_update(request_id, upper_bound_bytes)
                .map_err(|error| outbox_reservation_operation_error(request_id, error))?;
            #[cfg(test)]
            if FAIL_OUTBOUND_STAGING_COPY.with(std::cell::Cell::get) {
                return Err(super::OperationError::operation_resource_exhausted(
                    request_id,
                    "pendingOutboxUpdateBytes",
                    "injected outbound staging copy allocation failure",
                ));
            }
            let mut staged = Vec::new();
            staged.try_reserve_exact(update_v1.len()).map_err(|_| {
                super::OperationError::operation_resource_exhausted(
                    request_id,
                    "pendingOutboxUpdateBytes",
                    "captured outbound update could not allocate its staging copy",
                )
            })?;
            staged.extend_from_slice(update_v1);
            target.staged = Some((reservation, staged));
        }
        Ok(())
    }

    /// Infallible post-commit append of the staged update. No-op when
    /// detached or when the commit reserved nothing.
    pub(crate) fn commit_staged(&mut self) {
        if let Some(target) = self.target.as_mut() {
            if let Some((reservation, update)) = target.staged.take() {
                target.outbox.install(reservation, update);
            }
        }
    }
}

/// Frozen error mapping for pre-write outbox reservation failures:
/// deterministic ceiling saturation is `OPERATION_LIMIT_EXCEEDED` on the
/// configured collaboration-limit field; storage-reservation failure is the
/// allocation-class `OPERATION_RESOURCE_EXHAUSTED`.
fn outbox_reservation_operation_error(
    request_id: u64,
    error: crate::collaboration_runtime::outbox::OutboxReservationError,
) -> super::OperationError {
    use crate::collaboration_runtime::outbox::OutboxReservationError;
    match error {
        OutboxReservationError::Saturated {
            field,
            limit,
            actual,
        } => super::OperationError::operation_limit_exceeded(
            request_id,
            None,
            field,
            u64::try_from(limit).unwrap_or(u64::MAX),
            u64::try_from(actual).unwrap_or(u64::MAX),
        ),
        OutboxReservationError::Allocation => super::OperationError::operation_resource_exhausted(
            request_id,
            "pendingOutboxReservation",
            "collaboration outbox reservation could not allocate storage",
        ),
    }
}

// The methods in this block carrying `#[allow(dead_code)]` are the engine's
// plain convenience surface and test-support probes: they are exercised by
// crate tests and the cfg(test) bridge/document-api test support, while
// production entry points reach the same behavior through the
// `_with_outbox`/prepared variants used by `ffi_v2`. The constructors and the
// production seams in this block are genuinely live and carry no allow.
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

    pub fn plan_command(
        &self,
        request_id: u64,
        command: super::TypedCommand,
    ) -> super::OperationResult<super::CommandPlan> {
        self.plan_command_internal(request_id, command, None)
    }

    fn plan_command_internal<'a>(
        &'a self,
        request_id: u64,
        command: super::TypedCommand,
        preparation: Option<&'a std::cell::RefCell<Option<super::commands::PreparedCommandProof>>>,
    ) -> super::OperationResult<super::CommandPlan> {
        self.plan_command_internal_at_selection(
            request_id,
            command,
            preparation,
            None,
            None,
            super::TransactionOrigin::LocalCommand,
        )
    }

    fn plan_command_internal_at_selection<'a>(
        &'a self,
        request_id: u64,
        command: super::TypedCommand,
        preparation: Option<&'a std::cell::RefCell<Option<super::commands::PreparedCommandProof>>>,
        selection: Option<&'a super::ResolvedSelection>,
        initial_selection: Option<&'a super::SelectionInput>,
        origin: super::TransactionOrigin,
    ) -> super::OperationResult<super::CommandPlan> {
        let state = self
            .derived_state
            .as_ref()
            .ok_or_else(|| super::OperationError::engine_not_ready(request_id))?;
        let allow_deferred_admission =
            preparation.is_some() && state.mutation_lookup_seed.is_unavailable() && {
                let canonical_fingerprint = state.canonical_artifact.sha256();
                let canonical_serialized_len = state.canonical_artifact.serialized_len();
                state.matches_materialized_mutation_identity(
                    &state.canonical_artifact,
                    canonical_fingerprint,
                    canonical_serialized_len,
                    &self.resource_limits,
                    &self.schema_fingerprint,
                    self.revision,
                    self.state_revision,
                    self.yrs_state_epoch,
                )
            };
        super::commands::plan(
            super::commands::PlanningContext {
                request_id,
                revision: self.revision,
                state_revision: self.state_revision,
                document: &state.document,
                position_map: &state.position_map,
                rendered_text: &state.rendered_text,
                selection: selection.unwrap_or(&state.resolved_selection),
                initial_selection,
                origin,
                stored_marks: state.stored_marks.as_deref(),
                schema: &self.schema,
                resource_limits: &self.resource_limits,
                editing_limits: &self.editing_limits,
                max_length: self.max_length,
                yrs_state_epoch: self.yrs_state_epoch,
                canonical_schema: &self.canonical_schema,
                canonical_artifact: &state.canonical_artifact,
                allow_deferred_admission,
                preparation,
            },
            command,
        )
    }

    #[allow(dead_code)]
    pub fn apply_command(
        &mut self,
        request_id: u64,
        command: super::TypedCommand,
    ) -> super::OperationResult<Option<super::TypedTransactionResult>> {
        self.apply_command_with_sink(request_id, command, &mut OutboundUpdateSink::detached())
    }

    /// Production surface: [`Self::apply_command`] with an optionally attached
    /// collaboration outbox for outbound update capture.
    pub(crate) fn apply_command_with_outbox(
        &mut self,
        request_id: u64,
        command: super::TypedCommand,
        outbox: Option<&mut crate::collaboration_runtime::CollaborationOutbox>,
    ) -> super::OperationResult<Option<super::TypedTransactionResult>> {
        self.apply_command_with_sink(
            request_id,
            command,
            &mut OutboundUpdateSink::from_optional_outbox(outbox),
        )
    }

    pub(crate) fn apply_command_at_selection_with_outbox(
        &mut self,
        request_id: u64,
        command: super::TypedCommand,
        selection: super::SelectionInput,
        origin: super::TransactionOrigin,
        outbox: Option<&mut crate::collaboration_runtime::CollaborationOutbox>,
    ) -> super::OperationResult<Option<super::TypedTransactionResult>> {
        let resolved = self.resolve_selection_input_for_planning(request_id, &selection)?;
        let preparation = std::cell::RefCell::new(None);
        let mut outbound = OutboundUpdateSink::from_optional_outbox(outbox);
        let (_, result) = match self.plan_command_internal_at_selection(
            request_id,
            command,
            Some(&preparation),
            Some(&resolved),
            Some(&selection),
            origin,
        )? {
            super::CommandPlan::NotApplicable => return Ok(None),
            super::CommandPlan::SelectionOnly(transaction) => {
                let compiled = self.compile_typed_transaction(transaction)?;
                self.apply_compiled_transaction_with_history(compiled, true, None, &mut outbound)?
            }
            super::CommandPlan::Transaction(transaction) => {
                if let Some(proof) = preparation.into_inner() {
                    self.apply_prepared_command_transaction(
                        transaction,
                        proof,
                        true,
                        &mut outbound,
                    )?
                } else {
                    self.apply_typed_transaction_with_staged_context(
                        transaction,
                        true,
                        &mut outbound,
                    )?
                }
            }
        };
        result.map(Some).ok_or_else(|| {
            super::OperationError::engine_invariant_failed(
                request_id,
                None,
                "anchored command produced no result envelope",
            )
        })
    }

    fn resolve_selection_input_for_planning(
        &self,
        request_id: u64,
        selection: &super::SelectionInput,
    ) -> super::OperationResult<super::ResolvedSelection> {
        let state = self
            .derived_state
            .as_ref()
            .ok_or_else(|| super::OperationError::engine_not_ready(request_id))?;
        let resolve = |field: &'static str,
                       point: super::RevisionedPosition|
         -> super::OperationResult<u32> {
            super::position::editor_offset_to_doc_pos(
                point.offset,
                point.kind,
                &state.rendered_text,
                &state.position_map,
                &state.document,
            )
            .ok_or_else(|| {
                super::OperationError::selection_position_invalid(
                    request_id,
                    field,
                    format!("{field} is outside the current document"),
                )
            })
        };
        let resolved_point = |document: u32| -> super::OperationResult<super::ResolvedPoint> {
            let scalar = state.position_map.doc_to_scalar(document, &state.document);
            let utf16 = super::position::scalar_offset_to_utf16(&state.rendered_text, scalar)
                .ok_or_else(|| {
                    super::OperationError::engine_invariant_failed(
                        request_id,
                        None,
                        "resolved selection is not representable as UTF-16",
                    )
                })?;
            Ok(super::ResolvedPoint {
                document,
                scalar,
                utf16,
            })
        };
        match selection {
            super::SelectionInput::Text { anchor, head } => {
                let anchor = resolve("selection.anchor", *anchor)?;
                let head = resolve("selection.head", *head)?;
                let normalized =
                    Selection::text(anchor, head).normalized(&state.document, &state.position_map);
                let Selection::Text { anchor, head } = normalized else {
                    return Err(super::OperationError::engine_invariant_failed(
                        request_id,
                        None,
                        "text selection normalized to a non-text selection",
                    ));
                };
                Ok(super::ResolvedSelection::Text {
                    anchor: resolved_point(anchor)?,
                    head: resolved_point(head)?,
                })
            }
            super::SelectionInput::Node { at } => {
                let at = resolve("selection.at", *at)?;
                let Selection::Node { pos } =
                    Selection::node(at).normalized(&state.document, &state.position_map)
                else {
                    return Err(super::OperationError::selection_position_invalid(
                        request_id,
                        "selection.at",
                        "node selection did not resolve to a selectable node",
                    ));
                };
                if !selectable_void_at(state.document.root(), pos, 0, &self.schema) {
                    return Err(super::OperationError::selection_position_invalid(
                        request_id,
                        "selection.at",
                        "node selection must target a selectable void or atom node",
                    ));
                }
                Ok(super::ResolvedSelection::Node {
                    at: resolved_point(pos)?,
                })
            }
            super::SelectionInput::All => Ok(super::ResolvedSelection::All),
        }
    }

    fn apply_command_with_sink(
        &mut self,
        request_id: u64,
        command: super::TypedCommand,
        outbound: &mut OutboundUpdateSink<'_>,
    ) -> super::OperationResult<Option<super::TypedTransactionResult>> {
        let preparation = std::cell::RefCell::new(None);
        let (_, result) =
            match self.plan_command_internal(request_id, command, Some(&preparation))? {
                super::CommandPlan::NotApplicable => return Ok(None),
                super::CommandPlan::SelectionOnly(transaction) => {
                    let compiled = self.compile_typed_transaction(transaction)?;
                    self.apply_compiled_transaction_with_history(compiled, true, None, outbound)?
                }
                super::CommandPlan::Transaction(transaction) => {
                    if let Some(proof) = preparation.into_inner() {
                        self.apply_prepared_command_transaction(transaction, proof, true, outbound)?
                    } else {
                        self.apply_typed_transaction_with_staged_context(
                            transaction,
                            true,
                            outbound,
                        )?
                    }
                }
            };
        result.map(Some).ok_or_else(|| {
            super::OperationError::engine_invariant_failed(
                request_id,
                None,
                "rich prepared command produced no result envelope",
            )
        })
    }

    fn prepare_command_history_admission(
        &self,
        semantic: &PreparedSemanticAdmission,
    ) -> super::OperationResult<Option<super::prepared_admission::PreparedCommandHistoryAdmission>>
    {
        let transaction = semantic.transaction();
        let expected_document = semantic.expected_document();
        if transaction.history_policy == super::HistoryPolicy::Skip
            || expected_document
                == self.document().ok_or_else(|| {
                    super::OperationError::engine_not_ready(transaction.request_id)
                })?
        {
            return Ok(None);
        }
        let class = transaction.operations.iter().fold(
            super::compiler::HistoryClass::Skip,
            |class, operation| {
                use super::compiler::HistoryClass;
                let next = match operation {
                    super::TypedOperation::InsertText { .. }
                    | super::TypedOperation::InsertNode { .. } => HistoryClass::Insert,
                    super::TypedOperation::DeleteRange { .. } => HistoryClass::Delete,
                    super::TypedOperation::AddMark { .. }
                    | super::TypedOperation::RemoveMark { .. }
                    | super::TypedOperation::ReplaceMark { .. }
                    | super::TypedOperation::UpdateNodeAttrs { .. } => HistoryClass::Format,
                    _ => HistoryClass::Structural,
                };
                match (class, next) {
                    (HistoryClass::Skip, value) => value,
                    (left, right) if left == right => left,
                    _ => HistoryClass::Structural,
                }
            },
        );
        if class == super::compiler::HistoryClass::Skip {
            return Ok(None);
        }
        let state = self
            .derived_state
            .as_ref()
            .ok_or_else(|| super::OperationError::engine_not_ready(transaction.request_id))?;
        let candidate_artifact = semantic.canonical_artifact();
        let candidate_derivations = if let Some(derivations) = semantic.candidate_derivations() {
            derivations
        } else {
            let mut position_map =
                crate::position::PositionMap::build(expected_document, &self.schema);
            position_map.compact();
            let rendered_text = crate::render::rendered_text(expected_document, &self.schema);
            let rendered_scalars = u32::try_from(rendered_text.chars().count()).map_err(|_| {
                super::OperationError::engine_invariant_failed(
                    transaction.request_id,
                    None,
                    "prepared history rendered text exceeds the position domain",
                )
            })?;
            let mut document_text_bytes = 0usize;
            let mut stack = vec![expected_document.root()];
            while let Some(node) = stack.pop() {
                if let Some(text) = node.text_str() {
                    document_text_bytes =
                        document_text_bytes.checked_add(text.len()).ok_or_else(|| {
                            super::OperationError::engine_invariant_failed(
                                transaction.request_id,
                                None,
                                "prepared history text byte metric overflowed",
                            )
                        })?;
                }
                if let Some(content) = node.content() {
                    stack.extend(content.iter());
                }
            }
            super::compiler::CompiledDocumentDerivations {
                identity_seal: Arc::new(()),
                position_map,
                rendered_text,
                rendered_scalars,
                document_text_bytes,
                document_node_count: crate::editor_state::document_node_count(
                    expected_document.root(),
                ),
            }
        };
        let candidate_render = state
            .render_blocks
            .transition(
                &state.document,
                expected_document,
                &self.schema,
                &[],
                &self.resource_limits,
            )
            .map_err(|error| {
                super::OperationError::engine_invariant_failed(
                    transaction.request_id,
                    None,
                    format!("prepared history render transition failed: {error:?}"),
                )
            })?;
        let retained = history_document_snapshots_fit(
            state,
            expected_document,
            candidate_artifact,
            &candidate_derivations,
            &candidate_render.cache,
            state.stored_marks.as_deref(),
            &self.schema_fingerprint,
            &self.fragment_name,
            self.scope.as_ref(),
            self.editing_limits.max_derived_output_bytes,
        );
        let before = history_local_state(
            state,
            &self.fragment_name,
            self.scope.as_ref(),
            &self.resource_limits,
            &self.editing_limits,
            self.max_length,
            retained.map(|pair| pair.before),
        );
        let after = history_snapshot_template(
            candidate_artifact,
            state.stored_marks.as_deref(),
            &self.fragment_name,
            retained.map(|pair| pair.after),
        );
        let limits = self.history.pre_admit_capture_limits(
            transaction.request_id,
            transaction.origin,
            transaction.history_policy,
            class,
            semantic.undo_units(),
            before.metadata_bytes,
            after.metadata_bytes,
        )?;
        Ok(Some(
            super::prepared_admission::PreparedCommandHistoryAdmission {
                limits,
                before,
                after,
                candidate_derivations,
                candidate_render,
            },
        ))
    }

    fn prepare_execution_command_history_admission(
        &self,
        semantic: &super::prepared_admission::ExecutionSemanticAdmission,
    ) -> super::OperationResult<Option<super::prepared_admission::PreparedCommandHistoryAdmission>>
    {
        match semantic {
            super::prepared_admission::ExecutionSemanticAdmission::Eager(admission) => {
                self.prepare_command_history_admission(admission)
            }
            super::prepared_admission::ExecutionSemanticAdmission::Deferred(admission) => {
                self.prepare_deferred_command_history_admission(admission)
            }
        }
    }

    fn prepare_deferred_command_history_admission(
        &self,
        deferred: &super::prepared_admission::DeferredCommandAdmission,
    ) -> super::OperationResult<Option<super::prepared_admission::PreparedCommandHistoryAdmission>>
    {
        let transaction = deferred.transaction();
        let expected_document = deferred.expected_document();
        if transaction.history_policy == super::HistoryPolicy::Skip
            || expected_document
                == self.document().ok_or_else(|| {
                    super::OperationError::engine_not_ready(transaction.request_id)
                })?
        {
            return Ok(None);
        }
        let class = transaction.operations.iter().fold(
            super::compiler::HistoryClass::Skip,
            |class, operation| {
                use super::compiler::HistoryClass;
                let next = match operation {
                    super::TypedOperation::InsertText { .. }
                    | super::TypedOperation::InsertNode { .. } => HistoryClass::Insert,
                    super::TypedOperation::DeleteRange { .. } => HistoryClass::Delete,
                    super::TypedOperation::AddMark { .. }
                    | super::TypedOperation::RemoveMark { .. }
                    | super::TypedOperation::ReplaceMark { .. }
                    | super::TypedOperation::UpdateNodeAttrs { .. } => HistoryClass::Format,
                    _ => HistoryClass::Structural,
                };
                match (class, next) {
                    (HistoryClass::Skip, value) => value,
                    (left, right) if left == right => left,
                    _ => HistoryClass::Structural,
                }
            },
        );
        if class == super::compiler::HistoryClass::Skip {
            return Ok(None);
        }
        let state = self
            .derived_state
            .as_ref()
            .ok_or_else(|| super::OperationError::engine_not_ready(transaction.request_id))?;
        let evidence = deferred.prepare_history_evidence()?;
        let generic_render = || {
            state.render_blocks.transition(
                &state.document,
                expected_document,
                &self.schema,
                &[],
                &self.resource_limits,
            )
        };
        crate::render::incremental::record_localized_render_transition_attempt();
        let specialized = deferred.prepare_history_render_transition(
            state,
            &evidence.candidate_derivations,
            &self.schema,
            &self.resource_limits,
            &self.editing_limits,
            self.max_length,
            &self.schema_fingerprint,
        );
        let candidate_render = match specialized {
            Some(Ok(transition)) => {
                crate::render::incremental::record_localized_render_transition_success();
                Ok(transition)
            }
            Some(Err(_)) | None => {
                crate::render::incremental::record_localized_render_transition_fallback();
                generic_render()
            }
        }
        .map_err(|error| {
            super::OperationError::engine_invariant_failed(
                transaction.request_id,
                None,
                format!("prepared history render transition failed: {error:?}"),
            )
        })?;
        let retained = history_document_snapshots_fit_with_precomputed_after_charge(
            state,
            evidence.canonical_retained_bytes,
            evidence.source_document_retained_bytes,
            &evidence.candidate_derivations,
            &candidate_render.cache,
            state.stored_marks.as_deref(),
            &self.schema_fingerprint,
            &self.fragment_name,
            self.scope.as_ref(),
            self.editing_limits.max_derived_output_bytes,
        );
        let before = history_local_state(
            state,
            &self.fragment_name,
            self.scope.as_ref(),
            &self.resource_limits,
            &self.editing_limits,
            self.max_length,
            retained.map(|pair| pair.before),
        );
        let after = history_snapshot_template_from_identity(
            evidence.canonical_text_scalar_len,
            evidence.canonical_fingerprint,
            evidence.canonical_serialized_len,
            state.stored_marks.as_deref(),
            &self.fragment_name,
            retained.map(|pair| pair.after),
        );
        let limits = self.history.pre_admit_capture_limits(
            transaction.request_id,
            transaction.origin,
            transaction.history_policy,
            class,
            deferred.undo_units(),
            before.metadata_bytes,
            after.metadata_bytes,
        )?;
        Ok(Some(
            super::prepared_admission::PreparedCommandHistoryAdmission {
                limits,
                before,
                after,
                candidate_derivations: evidence.candidate_derivations,
                candidate_render,
            },
        ))
    }

    fn compiled_command_matches_proof(
        &self,
        compiled: &CompiledTransaction,
        document: &Document,
        selection: &Selection,
    ) -> super::OperationResult<bool> {
        let current_selection = self
            .derived_state
            .as_ref()
            .map(DerivedStateCache::legacy_selection)
            .ok_or_else(|| super::OperationError::engine_not_ready(compiled.request_id))?;
        let compiled_selection = match &compiled.selection_plan {
            SelectionPlan::Preserve => current_selection,
            SelectionPlan::Mapped(selection) | SelectionPlan::Explicit(selection) => {
                selection.clone()
            }
        };
        Ok(compiled.preview == *document && compiled_selection == *selection)
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    #[allow(dead_code)]
    pub fn undo(
        &mut self,
        request_id: u64,
    ) -> super::OperationResult<Option<super::TransactionCommit>> {
        Ok(self
            .apply_history_pop(request_id, true, false, &mut OutboundUpdateSink::detached())?
            .map(|(commit, _)| commit))
    }

    #[allow(dead_code)]
    pub fn redo(
        &mut self,
        request_id: u64,
    ) -> super::OperationResult<Option<super::TransactionCommit>> {
        Ok(self
            .apply_history_pop(
                request_id,
                false,
                false,
                &mut OutboundUpdateSink::detached(),
            )?
            .map(|(commit, _)| commit))
    }

    /// Production surface: [`Self::undo`] with an optionally attached
    /// collaboration outbox. The pop's outbound update is captured on the
    /// prepared candidate and reserved before the infallible install.
    pub(crate) fn undo_with_outbox(
        &mut self,
        request_id: u64,
        outbox: Option<&mut crate::collaboration_runtime::CollaborationOutbox>,
    ) -> super::OperationResult<Option<super::TransactionCommit>> {
        Ok(self
            .apply_history_pop(
                request_id,
                true,
                false,
                &mut OutboundUpdateSink::from_optional_outbox(outbox),
            )?
            .map(|(commit, _)| commit))
    }

    /// Production surface: [`Self::redo`] with an optionally attached
    /// collaboration outbox.
    pub(crate) fn redo_with_outbox(
        &mut self,
        request_id: u64,
        outbox: Option<&mut crate::collaboration_runtime::CollaborationOutbox>,
    ) -> super::OperationResult<Option<super::TransactionCommit>> {
        Ok(self
            .apply_history_pop(
                request_id,
                false,
                false,
                &mut OutboundUpdateSink::from_optional_outbox(outbox),
            )?
            .map(|(commit, _)| commit))
    }

    #[allow(dead_code)]
    pub fn undo_with_result(
        &mut self,
        request_id: u64,
    ) -> super::OperationResult<Option<super::TypedTransactionResult>> {
        Ok(self
            .apply_history_pop(request_id, true, true, &mut OutboundUpdateSink::detached())?
            .and_then(|(_, result)| result))
    }

    #[allow(dead_code)]
    pub fn redo_with_result(
        &mut self,
        request_id: u64,
    ) -> super::OperationResult<Option<super::TypedTransactionResult>> {
        Ok(self
            .apply_history_pop(request_id, false, true, &mut OutboundUpdateSink::detached())?
            .and_then(|(_, result)| result))
    }

    /// Merge one standard Yjs/Yrs Update-v1 into the authoritative document.
    ///
    /// The update is fully decoded, applied, derived, validated, and admitted
    /// on an isolated candidate before the live CRDT is opened for mutation.
    /// Remote-origin structs remain outside the local undo scope.
    ///
    /// This is exactly [`Self::prepare_remote_update_internal`] followed by
    /// [`Self::commit_prepared_remote_update_internal`], so the one-shot and
    /// prepare/commit paths cannot drift.
    #[allow(dead_code)]
    pub fn apply_remote_update_v1(
        &mut self,
        request_id: u64,
        update: &[u8],
    ) -> super::OperationResult<EngineCommit> {
        let prepared = self.prepare_remote_update_internal(request_id, update)?;
        self.commit_prepared_remote_update_internal(prepared)
    }

    /// Everything fallible in the remote-update pipeline, up to (and
    /// excluding) the live installation. Preparation is observationally pure:
    /// dependency classification produces an owned candidate while the live
    /// quarantine remains untouched until commit.
    fn prepare_remote_update_internal(
        &mut self,
        request_id: u64,
        update: &[u8],
    ) -> super::OperationResult<PreparedRemoteUpdate> {
        let admitted_revision = self.revision;
        let admitted_state_revision = self.state_revision;
        let admitted_epoch = self.yrs_state_epoch;
        admit_max_encoded_state_len(
            request_id,
            update.len(),
            self.resource_limits.max_encoded_state_bytes,
        )?;
        preflight_update_v1(update, &self.resource_limits)
            .map_err(|error| remote_ingress_error(request_id, error))?;
        let incoming_update = Update::decode_v1(update).map_err(|error| {
            super::OperationError::document_invalid(
                request_id,
                None,
                "update",
                format!("invalid Update-v1: {error}"),
            )
        })?;
        let (candidate_update, merged_update_bytes) = if let Some(quarantined) =
            self.quarantined_remote_update.as_deref()
        {
            let combined_len = quarantined.len().checked_add(update.len()).ok_or_else(|| {
                super::OperationError::document_limit_exceeded(
                    request_id,
                    None,
                    "maxEncodedStateBytes",
                    self.resource_limits.max_encoded_state_bytes as u64,
                    u64::MAX,
                )
            })?;
            admit_max_encoded_state_len(
                request_id,
                combined_len,
                self.resource_limits.max_encoded_state_bytes,
            )?;
            let quarantined_update = Update::decode_v1(quarantined).map_err(|error| {
                super::OperationError::engine_invariant_failed(
                    request_id,
                    None,
                    format!("quarantined Update-v1 cannot decode: {error}"),
                )
            })?;
            let merged = Update::merge_updates(vec![quarantined_update, incoming_update]);
            let merged_bytes = merged.encode_v1();
            admit_max_encoded_state_len(
                request_id,
                merged_bytes.len(),
                self.resource_limits.max_encoded_state_bytes,
            )?;
            preflight_update_v1(&merged_bytes, &self.resource_limits)
                .map_err(|error| remote_ingress_error(request_id, error))?;
            (merged, Some(merged_bytes))
        } else {
            (incoming_update, None)
        };
        let current_encoded = encode_state_bounded(&self.doc, &self.resource_limits)
            .map_err(|error| history_operation_error(request_id, error))?;
        let candidate_doc = fresh_utf16_doc_excluding(&self.durable_client_ids, self.client_id());
        {
            let mut txn =
                candidate_doc.transact_mut_with(TransactionOrigin::RemoteSync.as_yrs_origin());
            if !current_encoded.is_empty() {
                let current = Update::decode_v1(&current_encoded).map_err(|error| {
                    super::OperationError::engine_invariant_failed(
                        request_id,
                        None,
                        format!("current encoded state cannot decode: {error}"),
                    )
                })?;
                txn.apply_update(current).map_err(|error| {
                    super::OperationError::engine_invariant_failed(
                        request_id,
                        None,
                        format!("candidate cannot seed current state: {error}"),
                    )
                })?;
            }
            txn.apply_update(candidate_update).map_err(|error| {
                super::OperationError::document_invalid(
                    request_id,
                    None,
                    "update",
                    format!("candidate rejected Update-v1: {error}"),
                )
            })?;
        }
        let candidate_has_missing_updates = {
            let txn = candidate_doc.transact();
            txn.has_missing_updates()
        };
        if candidate_has_missing_updates {
            let quarantined = if let Some(merged) = merged_update_bytes {
                merged
            } else {
                #[cfg(test)]
                if FAIL_QUARANTINED_UPDATE_RESERVATION.with(std::cell::Cell::get) {
                    return Err(super::OperationError::operation_resource_exhausted(
                        request_id,
                        "remoteUpdate",
                        "injected quarantined remote update reservation failure",
                    ));
                }
                let mut admitted = Vec::new();
                admitted.try_reserve_exact(update.len()).map_err(|error| {
                    super::OperationError::operation_resource_exhausted(
                        request_id,
                        "remoteUpdate",
                        format!("cannot reserve quarantined remote update: {error}"),
                    )
                })?;
                admitted.extend_from_slice(update);
                admitted
            };
            return Ok(self.seal_remote_outcome(
                request_id,
                PreparedDocumentOutcome::Unchanged,
                PreparedDependencyState::Retain(quarantined),
            ));
        }
        let candidate_encoded =
            encode_candidate_state_bounded(&candidate_doc, &self.resource_limits)
                .map_err(|error| history_operation_error(request_id, error))?;
        if candidate_encoded == current_encoded {
            return Ok(self.seal_remote_outcome(
                request_id,
                PreparedDocumentOutcome::Unchanged,
                PreparedDependencyState::Clear,
            ));
        }

        let codec = YrsDocumentCodec::new(&self.schema, &self.resource_limits);
        let candidate_json = {
            let txn = candidate_doc.transact();
            let fragment = txn
                .get_xml_fragment(self.fragment_name.as_str())
                .ok_or_else(|| {
                    super::OperationError::document_invalid(
                        request_id,
                        None,
                        "update",
                        "remote update does not contain the configured document fragment",
                    )
                })?;
            codec
                .read_json(&fragment, &txn)
                .map_err(|error| remote_engine_error(request_id, error))?
        };
        let candidate_document = from_prosemirror_json_with_limits(
            &candidate_json,
            &self.schema,
            UnknownTypeMode::Preserve,
            &self.resource_limits,
        )
        .map_err(|error| remote_json_error(request_id, error))?;
        let candidate_document = rehydrate_reserved_html_opaque(&candidate_document);
        DocumentValidator::validate(&candidate_document, &self.schema, &self.resource_limits)
            .map_err(|error| remote_validation_error(request_id, error))?;
        if let Some(limit) = self.max_length {
            let actual = candidate_document.root().text_content().chars().count();
            if actual > limit as usize {
                return Err(super::OperationError::document_limit_exceeded(
                    request_id,
                    None,
                    "maxLength",
                    u64::from(limit),
                    actual as u64,
                ));
            }
        }
        let canonical_artifact =
            self.canonical_schema
                .derive(&candidate_document)
                .map_err(|error| {
                    super::OperationError::engine_invariant_failed(
                        request_id,
                        None,
                        format!("remote document serialization failed: {error}"),
                    )
                })?;
        if canonical_artifact.serialized_len() > self.editing_limits.max_derived_output_bytes {
            return Err(super::OperationError::document_limit_exceeded(
                request_id,
                None,
                "maxDerivedOutputBytes",
                u64::try_from(self.editing_limits.max_derived_output_bytes).unwrap_or(u64::MAX),
                u64::try_from(canonical_artifact.serialized_len()).unwrap_or(u64::MAX),
            ));
        }
        let next_revision =
            checked_operation_increment(request_id, self.revision, "documentRevision")?;
        let next_state_revision =
            checked_operation_increment(request_id, self.state_revision, "stateRevision")?;
        let next_epoch =
            checked_operation_increment(request_id, self.yrs_state_epoch, "yrsStateEpoch")?;
        let next_state = {
            let txn = candidate_doc.transact();
            let fragment = txn
                .get_xml_fragment(self.fragment_name.as_str())
                .ok_or_else(|| {
                    super::OperationError::engine_invariant_failed(
                        request_id,
                        None,
                        "validated remote candidate lost its document fragment",
                    )
                })?;
            if let Some(current) = self.derived_state.as_ref() {
                let candidate_render_blocks = Arc::new(
                    crate::render::incremental::CachedRenderBlocks::build(
                        &candidate_document,
                        &self.schema,
                        &self.resource_limits,
                    )
                    .map_err(|error| {
                        cached_render_operation_error(request_id, &self.resource_limits, error)
                    })?,
                );
                let fallback = affinity_aware_mapped_selection(
                    &current.legacy_selection(),
                    &current.relative_selection,
                    &StepMap::empty(),
                    &candidate_document,
                    &self.schema,
                    None,
                );
                let mut next = current
                    .after_document_change(
                        candidate_document.clone(),
                        canonical_artifact.clone(),
                        &txn,
                        &fragment,
                        &self.schema,
                        &self.schema_fingerprint,
                        &self.resource_limits,
                        &self.editing_limits,
                        self.max_length,
                        Arc::clone(&candidate_render_blocks),
                        None,
                        &StepMap::empty(),
                        UpdateMode::Rebuild,
                        &[],
                        None,
                        Some(&fallback),
                        false,
                        None,
                        None,
                        None,
                        next_revision,
                        next_state_revision,
                        next_epoch,
                    )
                    .ok_or_else(|| {
                        super::OperationError::selection_position_invalid(
                            request_id,
                            "selection",
                            "local relative selection cannot resolve after remote update",
                        )
                    })?;
                next.stored_marks = match (&current.resolved_selection, &next.resolved_selection) {
                    (
                        super::ResolvedSelection::Text {
                            anchor: current_anchor,
                            head: current_head,
                        },
                        super::ResolvedSelection::Text {
                            anchor: next_anchor,
                            head: next_head,
                        },
                    ) if current.relative_selection == next.relative_selection
                        && current_anchor.document == current_head.document
                        && next_anchor.document == next_head.document =>
                    {
                        current.stored_marks.clone()
                    }
                    _ => stored_marks_after_selection_change(
                        current.stored_marks.as_deref(),
                        &current.resolved_selection,
                        &next.resolved_selection,
                        &next.document,
                        &self.schema,
                    ),
                };
                next
            } else {
                DerivedStateCache::initialize(
                    candidate_document.clone(),
                    canonical_artifact.clone(),
                    &txn,
                    &fragment,
                    &self.schema,
                    &self.resource_limits,
                    &self.editing_limits,
                    self.max_length,
                    &self.schema_fingerprint,
                    next_revision,
                    next_state_revision,
                    next_epoch,
                )
                .ok_or_else(|| {
                    super::OperationError::engine_invariant_failed(
                        request_id,
                        None,
                        "remote candidate cannot initialize derived state",
                    )
                })?
            }
        };
        let durable_client_ids = {
            let txn = candidate_doc.transact();
            txn.state_vector()
                .iter()
                .map(|(client, _)| client.get())
                .collect::<HashSet<_>>()
        };
        let accepted_update = {
            let current_state_vector = self.doc.transact().state_vector();
            candidate_doc
                .transact()
                .encode_state_as_update_v1(&current_state_vector)
        };
        preflight_update_v1(&accepted_update, &self.resource_limits)
            .map_err(|error| history_operation_error(request_id, error))?;
        let live_update = Update::decode_v1(&accepted_update).map_err(|error| {
            super::OperationError::engine_invariant_failed(
                request_id,
                None,
                format!("candidate-produced incremental update cannot decode: {error}"),
            )
        })?;
        #[cfg(test)]
        super::compiler::check_atomic_failpoint(
            request_id,
            super::compiler::AtomicFailpoint::RemoteHistoryAdmission,
        )?;
        // Replay retention charges remote work in encoded-byte units: this is
        // the exact admitted incremental payload, not redundant caller input.
        let replay_byte_units = u64::try_from(accepted_update.len())
            .unwrap_or(u64::MAX)
            .max(1);
        let history_admission = self.history.pre_admit_excluded(
            request_id,
            TransactionOrigin::RemoteSync,
            replay_byte_units,
            &current_encoded,
            accepted_update.len(),
        )?;
        if (self.revision, self.state_revision, self.yrs_state_epoch)
            != (admitted_revision, admitted_state_revision, admitted_epoch)
        {
            return Err(super::OperationError::engine_invariant_failed(
                request_id,
                None,
                "remote update engine state changed during candidate admission",
            ));
        }
        let prepared_live_seed = {
            let candidate_txn = candidate_doc.transact();
            let candidate_fragment = candidate_txn
                .get_xml_fragment(self.fragment_name.as_str())
                .ok_or_else(|| {
                    super::OperationError::engine_invariant_failed(
                        request_id,
                        None,
                        "accepted remote candidate lost its Yrs fragment before seed rebind",
                    )
                })?;
            let live_txn = self.doc.transact();
            let live_fragment = live_txn
                .get_xml_fragment(self.fragment_name.as_str())
                .ok_or_else(|| {
                    super::OperationError::engine_invariant_failed(
                        request_id,
                        None,
                        "accepted remote update lost its live Yrs fragment before seed rebind",
                    )
                })?;
            let prepared = next_state
                .mutation_lookup_seed
                .prepare_authoritative_store_rebind(
                    request_id,
                    &candidate_txn,
                    &candidate_fragment,
                    &next_state.document,
                    &next_state.canonical_artifact,
                    &self.resource_limits,
                    &self.editing_limits,
                    self.max_length,
                    &self.schema_fingerprint,
                    next_epoch,
                    next_revision,
                    &live_txn,
                    &live_fragment,
                )?;
            if !prepared.matches_canonical_artifact(&next_state.canonical_artifact)
                || !prepared.matches(
                    &live_txn,
                    &live_fragment,
                    &next_state.document,
                    &self.resource_limits,
                    &self.editing_limits,
                    self.max_length,
                    &self.schema_fingerprint,
                    next_epoch,
                    next_revision,
                )
            {
                return Err(super::OperationError::engine_invariant_failed(
                    request_id,
                    None,
                    "prepared authoritative-store mutation lookup seed is stale",
                ));
            }
            prepared
        };
        #[cfg(test)]
        super::observability::record_staged_seed_preparation();
        Ok(self.seal_remote_outcome(
            request_id,
            PreparedDocumentOutcome::Changed(Box::new(PreparedRemoteInstall {
                live_update,
                accepted_update,
                history_admission,
                next_state,
                prepared_live_seed,
                durable_client_ids,
                next_revision,
                next_state_revision,
                next_epoch,
            })),
            PreparedDependencyState::Clear,
        ))
    }

    /// Seals prepared document and dependency candidates to the engine state
    /// that admitted them without mutating that state.
    fn seal_remote_outcome(
        &self,
        request_id: u64,
        document: PreparedDocumentOutcome,
        dependencies: PreparedDependencyState,
    ) -> PreparedRemoteUpdate {
        PreparedRemoteUpdate {
            seal: PreparedRemoteSeal {
                request_id,
                sealed_doc: self.doc.clone(),
                admitted_revision: self.revision,
                admitted_state_revision: self.state_revision,
                admitted_epoch: self.yrs_state_epoch,
                sealed_generation: self.remote_seal_generation,
            },
            document,
            dependencies,
        }
    }

    /// The already-proven live installation of a prepared remote update.
    /// Rejects with `ENGINE_INVARIANT_FAILED` — leaving the engine untouched —
    /// if anything mutated the engine after preparation (local edit, another
    /// remote commit, snapshot restore, replacement, or a newly quarantined
    /// dependency payload).
    fn commit_prepared_remote_update_internal(
        &mut self,
        prepared: PreparedRemoteUpdate,
    ) -> super::OperationResult<EngineCommit> {
        let PreparedRemoteUpdate {
            seal,
            document,
            dependencies,
        } = prepared;
        let PreparedRemoteSeal {
            request_id,
            sealed_doc,
            admitted_revision,
            admitted_state_revision,
            admitted_epoch,
            sealed_generation,
        } = seal;
        if !Doc::ptr_eq(&sealed_doc, &self.doc)
            || (self.revision, self.state_revision, self.yrs_state_epoch)
                != (admitted_revision, admitted_state_revision, admitted_epoch)
            || self.remote_seal_generation != sealed_generation
        {
            return Err(super::OperationError::engine_invariant_failed(
                request_id,
                None,
                "prepared remote update is sealed to a superseded engine state",
            ));
        }
        let dependency_candidate = match dependencies {
            PreparedDependencyState::Clear => None,
            PreparedDependencyState::Retain(bytes) => Some(bytes),
        };
        match document {
            PreparedDocumentOutcome::Unchanged => {
                if self.quarantined_remote_update != dependency_candidate {
                    self.quarantined_remote_update = dependency_candidate;
                    self.remote_seal_generation = self.remote_seal_generation.wrapping_add(1);
                }
                Ok(EngineCommit {
                    changed: false,
                    revision: self.revision,
                })
            }
            PreparedDocumentOutcome::Changed(install) => {
                let PreparedRemoteInstall {
                    live_update,
                    accepted_update,
                    history_admission,
                    mut next_state,
                    prepared_live_seed,
                    durable_client_ids,
                    next_revision,
                    next_state_revision,
                    next_epoch,
                } = *install;
                {
                    let mut txn = self.doc.transact_mut_with(history_admission.yrs_origin());
                    txn.apply_update(live_update).expect(
                        "candidate-proved remote update must apply to identical live state",
                    );
                }
                next_state.mutation_lookup_seed = prepared_live_seed;
                self.history
                    .finish_prepared_excluded(history_admission, accepted_update);
                self.quarantined_remote_update = dependency_candidate;
                self.derived_state = Some(next_state);
                self.durable_client_ids = durable_client_ids;
                self.revision = next_revision;
                self.state_revision = next_state_revision;
                self.yrs_state_epoch = next_epoch;
                self.last_committed_origin = Some(TransactionOrigin::RemoteSync);
                self.document_origin = super::DocumentOrigin::RemoteCollaboration;
                self.prepared_candidate_cache = None;
                Ok(EngineCommit {
                    changed: true,
                    revision: self.revision,
                })
            }
        }
    }

    /// Production surface: admit a remote Update-v1 without installing it, so a
    /// coupled protocol reply can be reserved between preparation and
    /// installation. See [`Self::apply_remote_update_v1`], which is this
    /// method followed by [`Self::commit_prepared_remote_update`].
    pub fn prepare_remote_update_v1(
        &mut self,
        request_id: u64,
        update: &[u8],
    ) -> super::OperationResult<PreparedRemoteUpdate> {
        self.prepare_remote_update_internal(request_id, update)
    }

    /// Production surface: install a prepared remote update. One-shot; the
    /// prepared value is consumed whether or not installation is admitted.
    pub fn commit_prepared_remote_update(
        &mut self,
        prepared: PreparedRemoteUpdate,
    ) -> super::OperationResult<EngineCommit> {
        self.commit_prepared_remote_update_internal(prepared)
    }

    /// Production surface: the authoritative store's state vector, encoded v1.
    /// Read-only: no revision, epoch, state, or history effect.
    pub fn encode_state_vector_v1(&self, request_id: u64) -> super::OperationResult<Vec<u8>> {
        let encoded = self.doc.transact().state_vector().encode_v1();
        // Defensive symmetry with every other encoded artifact: a consistent
        // engine cannot produce a state vector above this ceiling (its full
        // encoded state is strictly larger and is bounded by the same limit
        // on every admission path), so the gate itself is unit-tested at the
        // exact/one-over boundary instead.
        admit_max_encoded_state_len(
            request_id,
            encoded.len(),
            self.resource_limits.max_encoded_state_bytes,
        )?;
        Ok(encoded)
    }

    /// Production surface: the incremental Update-v1 that brings a peer at
    /// `remote_state_vector_v1` up to the authoritative store. Read-only.
    /// Malformed input rejects with a structured error, bounded before any
    /// decode work by the encoded-state byte ceiling.
    pub fn encode_diff_v1(
        &self,
        request_id: u64,
        remote_state_vector_v1: &[u8],
    ) -> super::OperationResult<Vec<u8>> {
        admit_max_encoded_state_len(
            request_id,
            remote_state_vector_v1.len(),
            self.resource_limits.max_encoded_state_bytes,
        )?;
        let remote_state_vector =
            StateVector::decode_v1(remote_state_vector_v1).map_err(|error| {
                super::OperationError::document_invalid(
                    request_id,
                    None,
                    "stateVector",
                    format!("invalid state vector: {error}"),
                )
            })?;
        let diff = self
            .doc
            .transact()
            .encode_state_as_update_v1(&remote_state_vector);
        admit_max_encoded_state_len(
            request_id,
            diff.len(),
            self.resource_limits.max_encoded_state_bytes,
        )?;
        Ok(diff)
    }

    /// Production surface: the bounded structural classification half of the
    /// remote ingress pipeline, exactly the byte-ceiling admission plus
    /// Update-v1 preflight that [`Self::prepare_remote_update_v1`] runs
    /// first. Read-only; the protocol layer uses it to classify a rejected
    /// payload as malformed encoding (preflight also fails) versus
    /// admitted-but-inadmissible content (preflight passes).
    pub fn preflight_remote_update_v1(
        &self,
        request_id: u64,
        update: &[u8],
    ) -> super::OperationResult<()> {
        admit_max_encoded_state_len(
            request_id,
            update.len(),
            self.resource_limits.max_encoded_state_bytes,
        )?;
        preflight_update_v1(update, &self.resource_limits)
            .map_err(|error| remote_ingress_error(request_id, error))?;
        Ok(())
    }

    /// Production surface: byte accounting for the engine-owned dependency
    /// quarantine (`0` when no update is pending). The engine is the sole
    /// owner of the pending bytes themselves; the collaboration runtime
    /// charges this figure against its configured ceilings without ever
    /// holding a second payload copy.
    pub fn pending_remote_dependency_bytes(&self) -> usize {
        self.quarantined_remote_update
            .as_deref()
            .map_or(0, <[u8]>::len)
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

    fn apply_history_pop(
        &mut self,
        request_id: u64,
        undoing: bool,
        with_result: bool,
        outbound: &mut OutboundUpdateSink<'_>,
    ) -> super::OperationResult<
        Option<(
            super::TransactionCommit,
            Option<super::TypedTransactionResult>,
        )>,
    > {
        let Some(prepared) = self.prepare_history_pop(request_id, undoing, with_result)? else {
            return Ok(None);
        };
        self.commit_prepared_history_pop(prepared, outbound)
            .map(Some)
    }

    fn prepare_history_pop(
        &self,
        request_id: u64,
        undoing: bool,
        with_result: bool,
    ) -> super::OperationResult<Option<PreparedHistoryPop>> {
        if if undoing {
            !self.history.can_undo()
        } else {
            !self.history.can_redo()
        } {
            return Ok(None);
        }

        let next_document_revision =
            checked_operation_increment(request_id, self.revision, "documentRevision")?;
        let next_state_revision =
            checked_operation_increment(request_id, self.state_revision, "stateRevision")?;
        let next_yrs_state_epoch =
            checked_operation_increment(request_id, self.yrs_state_epoch, "yrsStateEpoch")?;

        let action = if undoing {
            super::history::HistoryAction::Undo
        } else {
            super::history::HistoryAction::Redo
        };
        let candidate_doc = self.new_history_candidate_doc();
        self.history.seed_candidate(request_id, &candidate_doc)?;
        let candidate_fragment =
            candidate_doc.get_or_insert_xml_fragment(self.fragment_name.as_str());
        let mut candidate_history =
            self.history
                .replay_into(request_id, &candidate_doc, &candidate_fragment)?;
        let candidate_pop = match action {
            super::history::HistoryAction::Undo => candidate_history.undo(),
            super::history::HistoryAction::Redo => candidate_history.redo(),
        };
        if !candidate_pop.changed {
            return Err(super::OperationError::engine_invariant_failed(
                request_id,
                None,
                "bounded history replay cannot reproduce the next live pop",
            ));
        }
        let restored_slot = candidate_pop.restored.as_ref().ok_or_else(|| {
            super::OperationError::engine_invariant_failed(
                request_id,
                None,
                "changed history candidate supplied no restoration metadata",
            )
        })?;
        let restored = restored_slot.get().ok_or_else(|| {
            super::OperationError::engine_invariant_failed(
                request_id,
                None,
                "changed history candidate supplied an unsealed restoration snapshot",
            )
        })?;
        let PreparedHistoryCandidateState {
            state: candidate_state,
            encoded_state: candidate_encoded_state,
            candidate_publication,
        } = self.derive_history_candidate_state(
            request_id,
            &candidate_doc,
            restored,
            next_document_revision,
            next_state_revision,
            next_yrs_state_epoch,
        )?;

        let mut result = with_result
            .then(|| self.prepare_history_result(request_id, &candidate_state))
            .transpose()?;

        candidate_history.accept_action(request_id, action, candidate_encoded_state)?;
        if let Some(result) = &mut result {
            result.history_state = crate::editor_state::HistoryState {
                can_undo: candidate_history.can_undo(),
                can_redo: candidate_history.can_redo(),
            };
        }
        Ok(Some(PreparedHistoryPop {
            request_id,
            candidate_doc,
            candidate_history,
            candidate_state,
            candidate_publication,
            next_document_revision,
            next_state_revision,
            next_yrs_state_epoch,
            result,
        }))
    }

    fn commit_prepared_history_pop(
        &mut self,
        mut prepared: PreparedHistoryPop,
        outbound: &mut OutboundUpdateSink<'_>,
    ) -> super::OperationResult<(
        super::TransactionCommit,
        Option<super::TypedTransactionResult>,
    )> {
        let ready_seed = {
            let txn = prepared.candidate_doc.transact();
            let fragment = txn
                .get_xml_fragment(self.fragment_name.as_str())
                .ok_or_else(|| {
                    super::OperationError::engine_invariant_failed(
                        prepared.request_id,
                        None,
                        "prepared history candidate lost its configured fragment",
                    )
                })?;
            let seed = if let Some(capability) = prepared.candidate_publication.take() {
                capability.prepare_candidate_publication(
                    prepared.request_id,
                    &txn,
                    &fragment,
                    &self.schema,
                    &prepared.candidate_state.document,
                    &prepared.candidate_state.canonical_artifact,
                    &self.resource_limits,
                    &self.editing_limits,
                    self.max_length,
                    &self.schema_fingerprint,
                    prepared.next_yrs_state_epoch,
                    prepared.next_document_revision,
                )?
            } else {
                Arc::clone(&prepared.candidate_state.mutation_lookup_seed)
            };
            if !seed.matches_canonical_artifact(&prepared.candidate_state.canonical_artifact)
                || !seed.matches(
                    &txn,
                    &fragment,
                    &prepared.candidate_state.document,
                    &self.resource_limits,
                    &self.editing_limits,
                    self.max_length,
                    &self.schema_fingerprint,
                    prepared.next_yrs_state_epoch,
                    prepared.next_document_revision,
                )
            {
                return Err(super::OperationError::engine_invariant_failed(
                    prepared.request_id,
                    None,
                    "prepared history candidate has no matching ready mutation seed",
                ));
            }
            seed
        };
        prepared.candidate_state.mutation_lookup_seed = ready_seed;
        if outbound.is_attached() {
            // Standard reconciliation update for the pop: the candidate's
            // new pop-authored structs against the live store's state vector
            // plus the candidate's complete delete set (redundant deletes are
            // idempotent for peers). It is captured on the fully prepared
            // candidate BEFORE the infallible install, so its exact length is
            // the admitted conservative bound (actual == bound).
            let outbound_update = {
                let live_state_vector = self.doc.transact().state_vector();
                prepared
                    .candidate_doc
                    .transact()
                    .encode_state_as_update_v1(&live_state_vector)
            };
            outbound.reserve_and_stage(
                prepared.request_id,
                outbound_update.len(),
                &outbound_update,
            )?;
        }
        let installed = self.install_prepared_history_pop(prepared);
        outbound.commit_staged();
        Ok(installed)
    }

    fn install_prepared_history_pop(
        &mut self,
        prepared: PreparedHistoryPop,
    ) -> (
        super::TransactionCommit,
        Option<super::TypedTransactionResult>,
    ) {
        // All recoverable candidate work is complete. Installation below is
        // infallible ownership transfer plus admitted scalar assignments.
        self.doc = prepared.candidate_doc;
        // Same client identity and logical session: awareness migrates every
        // live state (local and remote) with clocks intact.
        if let Some(awareness) = self.awareness.as_mut() {
            awareness.rebind_preserving_peers(&self.doc);
        }
        self.history = prepared.candidate_history;
        self.derived_state = Some(prepared.candidate_state);
        self.revision = prepared.next_document_revision;
        self.state_revision = prepared.next_state_revision;
        self.yrs_state_epoch = prepared.next_yrs_state_epoch;
        self.last_committed_origin = Some(TransactionOrigin::UndoRedo);
        self.document_origin = super::DocumentOrigin::History;
        self.prepared_candidate_cache = None;
        (
            super::TransactionCommit {
                request_id: prepared.request_id,
                changed: true,
                document_revision: self.revision,
                state_revision: self.state_revision,
                origin: TransactionOrigin::UndoRedo,
            },
            prepared.result,
        )
    }

    fn prepare_history_result(
        &self,
        request_id: u64,
        candidate: &DerivedStateCache,
    ) -> super::OperationResult<super::TypedTransactionResult> {
        let current = self
            .derived_state
            .as_ref()
            .ok_or_else(|| super::OperationError::engine_not_ready(request_id))?;
        let selection = candidate.resolved_selection.clone();
        let legacy_selection = candidate.legacy_selection();
        let commands = crate::editor_state::command_applicability(
            &candidate.document,
            &self.schema,
            &legacy_selection,
            &self.resource_limits,
        );
        let active_state = crate::editor_state::active_state(
            &candidate.document,
            &self.schema,
            &legacy_selection,
            candidate.stored_marks.as_deref(),
            commands,
            &self.resource_limits,
        );
        let render_update =
            cached_transition_render_update(&current.render_blocks.classify_transition_to(
                &current.document,
                &candidate.document,
                &candidate.render_blocks,
                &[],
            ));
        let result = super::TypedTransactionResult {
            request_id,
            origin: TransactionOrigin::UndoRedo,
            changed: true,
            document_revision: candidate.document_revision,
            state_revision: candidate.state_revision,
            selection,
            active_state,
            history_state: crate::editor_state::HistoryState {
                can_undo: self.can_undo(),
                can_redo: self.can_redo(),
            },
            render_update,
        };
        self.admit_typed_result(request_id, &result)?;
        Ok(result)
    }

    fn new_history_candidate_doc(&self) -> Doc {
        Doc::with_options(Options {
            client_id: self.doc.client_id(),
            guid: self.doc.guid(),
            offset_kind: OffsetKind::Utf16,
            // History StackItems reference deleted structs which are present in
            // the full update but whose live keep flags are not encoded.
            skip_gc: true,
            ..Options::default()
        })
    }

    fn derive_history_candidate_state(
        &self,
        request_id: u64,
        doc: &Doc,
        restored: &super::history::HistoryLocalState,
        document_revision: u64,
        state_revision: u64,
        yrs_state_epoch: u64,
    ) -> super::OperationResult<PreparedHistoryCandidateState> {
        let txn = doc.transact();
        let fragment = txn
            .get_xml_fragment(self.fragment_name.as_str())
            .ok_or_else(|| {
                super::OperationError::engine_invariant_failed(
                    request_id,
                    None,
                    "history result fragment is missing",
                )
            })?;
        let (derived_json, history_admission) =
            if let Some(document_snapshot) = restored.document_snapshot.as_deref() {
                document_snapshot
                    .prepare_candidate_read(
                        request_id,
                        &txn,
                        &fragment,
                        &self.schema,
                        &self.resource_limits,
                        &self.editing_limits,
                        self.max_length,
                        &self.schema_fingerprint,
                        &self.fragment_name,
                        self.scope.as_ref(),
                        yrs_state_epoch,
                        document_revision,
                    )?
                    .into_parts()
            } else {
                let json = YrsDocumentCodec::new(&self.schema, &self.resource_limits)
                    .read_json(&fragment, &txn)
                    .map_err(|error| history_operation_error(request_id, error))?;
                (json, None)
            };
        crate::transform::validate_input_mark_set(
            restored.stored_marks.as_deref().unwrap_or_default(),
            &self.schema,
        )
        .map_err(|error| {
            super::OperationError::engine_invariant_failed(
                request_id,
                None,
                format!("history metadata contains invalid stored marks: {error}"),
            )
        })?;
        if let (Some(document_snapshot), Some(admission)) =
            (restored.document_snapshot.as_deref(), history_admission)
        {
            let restored_state = DerivedStateCache::restore_history_document_snapshot(
                request_id,
                document_snapshot,
                admission,
                &txn,
                &fragment,
                &self.schema,
                &restored.relative_selection,
                &restored.resolved_selection,
                restored.stored_marks.clone(),
                &self.resource_limits,
                &self.editing_limits,
                self.max_length,
                &self.schema_fingerprint,
                document_revision,
                state_revision,
                yrs_state_epoch,
            )?;
            if let Some(restored) = restored_state {
                drop(txn);
                let encoded = encode_state_bounded(doc, &self.resource_limits)
                    .map_err(|error| history_operation_error(request_id, error))?;
                return Ok(PreparedHistoryCandidateState {
                    state: restored.state,
                    encoded_state: encoded,
                    candidate_publication: Some(restored.candidate_publication),
                });
            }
        }
        let document = from_prosemirror_json_with_limits(
            &derived_json,
            &self.schema,
            UnknownTypeMode::Preserve,
            &self.resource_limits,
        )
        .map_err(|error| {
            super::OperationError::document_invalid(request_id, None, "document", error.to_string())
        })?;
        let document =
            canonicalize_yrs_document(&rehydrate_reserved_html_opaque(&document), &self.schema);
        DocumentValidator::validate(&document, &self.schema, &self.resource_limits).map_err(
            |error| {
                if error.code() == "DOCUMENT_LIMIT_EXCEEDED" {
                    super::OperationError::document_limit_exceeded(
                        request_id,
                        None,
                        "document",
                        error.limit.unwrap_or(0) as u64,
                        error.actual.unwrap_or(0) as u64,
                    )
                } else {
                    super::OperationError::document_invalid(
                        request_id,
                        None,
                        "document",
                        error.to_string(),
                    )
                }
            },
        )?;
        if let Some(limit) = self.max_length {
            let actual = document.root().text_content().chars().count() as u64;
            if actual > u64::from(limit) {
                return Err(super::OperationError::document_limit_exceeded(
                    request_id,
                    None,
                    "maxLength",
                    u64::from(limit),
                    actual,
                ));
            }
        }
        let canonical_artifact = self.canonical_schema.derive(&document).map_err(|error| {
            super::OperationError::engine_invariant_failed(
                request_id,
                None,
                format!("history result serialization failed: {error}"),
            )
        })?;
        if canonical_artifact.serialized_len() > self.editing_limits.max_derived_output_bytes {
            return Err(super::OperationError::document_limit_exceeded(
                request_id,
                None,
                "maxDerivedOutputBytes",
                u64::try_from(self.editing_limits.max_derived_output_bytes).unwrap_or(u64::MAX),
                u64::try_from(canonical_artifact.serialized_len()).unwrap_or(u64::MAX),
            ));
        }
        let canonical_fingerprint = canonical_artifact.sha256();
        // Yrs recreates inserted structs with new IDs during redo, so the
        // original relative cursor can remain valid yet resolve beside the
        // redone content. Preserve the document-relative snapshot as the CRDT
        // metadata and reseal it from the exact resolved fallback on restore.
        let restored_relative = if canonical_fingerprint == restored.canonical_fingerprint {
            history_selection_to_relative(
                &txn,
                &fragment,
                &restored.relative_selection,
                &restored.resolved_selection,
                &self.schema,
            )
            .ok_or_else(|| {
                super::OperationError::engine_invariant_failed(
                    request_id,
                    None,
                    "history selection affinity is not exactly representable in the candidate",
                )
            })?
        } else {
            restored.relative_selection.clone()
        };
        let stored_marks = restored
            .stored_marks
            .as_deref()
            .map(|marks| super::derived_state::canonical_marks(marks, &self.schema));
        let state = DerivedStateCache::initialize_history(
            document,
            canonical_artifact,
            &txn,
            &fragment,
            &self.schema,
            &self.resource_limits,
            &self.editing_limits,
            self.max_length,
            &self.schema_fingerprint,
            restored_relative,
            stored_marks,
            document_revision,
            state_revision,
            yrs_state_epoch,
        )
        .ok_or_else(|| {
            super::OperationError::engine_invariant_failed(
                request_id,
                None,
                "history result cannot initialize derived editor state or resolve restoration selection",
            )
        })?;
        if !state
            .mutation_lookup_seed
            .matches_canonical_artifact(&state.canonical_artifact)
            || !state.mutation_lookup_seed.matches(
                &txn,
                &fragment,
                &state.document,
                &self.resource_limits,
                &self.editing_limits,
                self.max_length,
                &self.schema_fingerprint,
                yrs_state_epoch,
                document_revision,
            )
        {
            return Err(super::OperationError::engine_invariant_failed(
                request_id,
                None,
                "generic history candidate has no matching ready mutation seed",
            ));
        }
        drop(txn);
        let encoded = encode_state_bounded(doc, &self.resource_limits)
            .map_err(|error| history_operation_error(request_id, error))?;
        Ok(PreparedHistoryCandidateState {
            state,
            encoded_state: encoded,
            candidate_publication: None,
        })
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

    #[allow(dead_code)] // exposes the internal compiler through atomic application.
    pub(crate) fn compile_typed_transaction(
        &self,
        transaction: super::TypedTransaction,
    ) -> super::OperationResult<CompiledTransaction> {
        self.compile_typed_transaction_internal(transaction, None)
    }

    #[cfg(test)]
    fn compile_finalized_prepared_typed_transaction(
        &self,
        transaction: super::TypedTransaction,
        semantic_admission: &PreparedSemanticAdmission,
        proof_document: &Document,
        proof_selection: &Selection,
        candidate_derivations: Option<&super::compiler::CompiledDocumentDerivations>,
    ) -> super::OperationResult<CompiledTransaction> {
        let state = self.derived_state.as_ref().ok_or_else(|| {
            super::OperationError::engine_invariant_failed(
                transaction.request_id,
                None,
                "ready Yrs engine has no derived state",
            )
        })?;
        let txn = self.doc.transact();
        let fragment = txn
            .get_xml_fragment(self.fragment_name.as_str())
            .ok_or_else(|| {
                super::OperationError::engine_invariant_failed(
                    transaction.request_id,
                    None,
                    "ready Yrs document fragment is missing",
                )
            })?;
        let authority = super::prepared_admission::InstalledDerivedStateAuthority::new(state);
        self.compile_finalized_prepared_typed_transaction_with_read_view(
            transaction,
            semantic_admission,
            proof_document,
            proof_selection,
            candidate_derivations,
            &authority,
            &txn,
            &fragment,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn compile_finalized_prepared_typed_transaction_with_read_view<T: ReadTxn>(
        &self,
        transaction: super::TypedTransaction,
        semantic_admission: &PreparedSemanticAdmission,
        proof_document: &Document,
        proof_selection: &Selection,
        candidate_derivations: Option<&super::compiler::CompiledDocumentDerivations>,
        authority: &dyn super::prepared_admission::DerivedStateAuthority,
        txn: &T,
        fragment: &XmlFragmentRef,
    ) -> super::OperationResult<CompiledTransaction> {
        let mut compiled = self.compile_typed_transaction_with_read_view(
            transaction,
            Some((semantic_admission, proof_document)),
            authority,
            txn,
            fragment,
        )?;
        if compiled.preview != *proof_document {
            return Err(super::OperationError::engine_invariant_failed(
                compiled.request_id,
                None,
                "prepared command compiler diverged from its simulated document",
            ));
        }
        if let Some(derivations) = candidate_derivations {
            compiled.preview = proof_document.clone();
            compiled.preview_derivations = Some(derivations.clone());
        }
        let state = authority.installed();
        let eligible_admission = compiled
            .localized_insert_admission
            .as_ref()
            .filter(|admission| {
                let current_at_insertion = matches!(
                    &state.resolved_selection,
                    super::ResolvedSelection::Text { anchor, head }
                        if anchor == head
                            && anchor.document == admission.inserted_document_position()
                );
                let operation_result = admission.operation_result_selection();
                let operation_result_legacy =
                    super::derived_state::resolved_to_legacy(operation_result);
                compiled.origin == super::TransactionOrigin::LocalCommand
                    && compiled.history_policy == super::HistoryPolicy::Boundary
                    && compiled.history_class == super::compiler::HistoryClass::Insert
                    && compiled.localized_semantic_used
                    && admission.inserted_scalars() > 0
                    && current_at_insertion
                    && matches!(
                        &compiled.selection_plan,
                        SelectionPlan::Explicit(selection)
                            if *selection == operation_result_legacy
                                && *selection == *proof_selection
                    )
                    && compiled.relative_selection_plan == RelativeSelectionPlan::OperationResult
                    && matches!(
                        &compiled.stored_marks_plan,
                        StoredMarksPlan::Set(stored_marks)
                            if *stored_marks == state.stored_marks
                    )
                    && compiled.preview == *proof_document
                    && *operation_result
                        == super::derived_state::resolved_from_legacy_with_view(
                            &compiled.preview,
                            &operation_result_legacy,
                            &self.schema,
                            compiled
                                .preview_derivations
                                .as_ref()
                                .map(|derivations| &derivations.position_map)
                                .unwrap_or(&state.position_map),
                            compiled
                                .preview_derivations
                                .as_ref()
                                .map(|derivations| derivations.rendered_text.as_str())
                                .unwrap_or(state.rendered_text.as_str()),
                        )
                        .unwrap_or(super::ResolvedSelection::All)
            });
        let transition = eligible_admission
            .map(|admission| {
                let StoredMarksPlan::Set(stored_marks) = &compiled.stored_marks_plan else {
                    unreachable!("eligible active-state transition has sealed marks")
                };
                state.prepare_active_state_transition(
                    compiled.request_id,
                    authority,
                    admission,
                    &compiled.preview,
                    admission.operation_result_selection(),
                    stored_marks.as_deref(),
                    &self.resource_limits,
                    &self.editing_limits,
                    self.max_length,
                    self.yrs_state_epoch,
                )
            })
            .transpose()?;
        compiled.prepared_selection_state = eligible_admission.and_then(|admission| {
            super::derived_state::record_prewrite_selection_proof_attempt();
            let prepared = self.materialize_prewrite_selection_state(&compiled, admission, txn);
            if prepared.is_some() {
                super::derived_state::record_prewrite_selection_proof_finalization();
            } else {
                super::derived_state::record_prewrite_selection_proof_fallback();
            }
            prepared
        });
        compiled.prepared_selection_mutation_seal = compiled
            .prepared_selection_state
            .as_ref()
            .and_then(|_| super::compiler::PreparedSelectionMutationSeal::capture(&compiled));
        compiled.prepared_active_state_transition = transition;
        Ok(compiled)
    }

    #[cfg(test)]
    fn compile_prepared_typed_transaction(
        &self,
        transaction: super::TypedTransaction,
        proof: super::commands::PreparedCommandProof,
    ) -> super::OperationResult<CompiledTransaction> {
        let super::commands::PreparedCommandProof {
            document,
            selection,
            execution_admission,
        } = proof;
        let semantic_admission = match execution_admission {
            super::prepared_admission::ExecutionSemanticAdmission::Eager(admission) => admission,
            super::prepared_admission::ExecutionSemanticAdmission::Deferred(admission) => {
                admission.into_eager()?
            }
        };
        self.compile_finalized_prepared_typed_transaction(
            transaction,
            &semantic_admission,
            &document,
            &selection,
            None,
        )
    }

    fn materialize_prewrite_selection_state<T: ReadTxn>(
        &self,
        compiled: &CompiledTransaction,
        admission: &super::derived_state::LocalizedInsertAdmission,
        txn: &T,
    ) -> Option<FinalizedSelectionState> {
        let state = self.derived_state.as_ref()?;
        let current_at_insertion = matches!(
            &state.resolved_selection,
            super::ResolvedSelection::Text { anchor, head }
                if anchor == head
                    && anchor.document == admission.inserted_document_position()
        );
        if compiled.origin != super::TransactionOrigin::LocalCommand
            || compiled.history_policy != super::HistoryPolicy::Boundary
            || compiled.history_class != super::compiler::HistoryClass::Insert
            || !compiled.localized_semantic_used
            || admission.inserted_scalars() == 0
            || !current_at_insertion
            || compiled.base_state_revision != state.state_revision
            || compiled.yrs_state_epoch != self.yrs_state_epoch
            || !matches!(
                &compiled.stored_marks_plan,
                StoredMarksPlan::Set(stored_marks) if *stored_marks == state.stored_marks
            )
        {
            return None;
        }
        let [YrsMutationAction::InsertText {
            target,
            index_utf16,
            len_utf16,
            signature,
            operation_index,
            ..
        }] = compiled.mutation_plan.actions.as_slice()
        else {
            return None;
        };
        if *operation_index != 0
            || *len_utf16 == 0
            || *len_utf16 != admission.inserted_utf16()
            || *index_utf16 == 0
            || *index_utf16 >= signature.initial_len_utf16()
        {
            return None;
        }
        let sticky = target.sticky_index(txn, *index_utf16, Assoc::After)?;
        let offset = sticky.get_offset(txn)?;
        let exact_target = yrs::branch::BranchPtr::from(<yrs::types::xml::XmlTextRef as AsRef<
            yrs::branch::Branch,
        >>::as_ref(target));
        if offset.index != *index_utf16 || offset.branch != exact_target {
            return None;
        }
        let point = super::RelativePoint {
            sticky,
            affinity: super::Affinity::After,
        };
        let relative = super::RelativeSelection::Text {
            anchor: point.clone(),
            head: point,
        };
        let resolved = admission.operation_result_selection().clone();
        let legacy = super::derived_state::resolved_to_legacy(&resolved);
        if !matches!(
            &compiled.selection_plan,
            SelectionPlan::Explicit(selection) if *selection == legacy
        ) || compiled.relative_selection_plan != RelativeSelectionPlan::OperationResult
        {
            return None;
        }
        let preview_derivations = compiled.preview_derivations.as_ref()?;
        if resolved
            != super::derived_state::resolved_from_legacy_with_view(
                &compiled.preview,
                &legacy,
                &self.schema,
                &preview_derivations.position_map,
                &preview_derivations.rendered_text,
            )?
        {
            return None;
        }
        FinalizedSelectionState::new(relative, resolved, legacy)
    }

    fn compile_typed_transaction_internal(
        &self,
        transaction: super::TypedTransaction,
        prepared_semantics: Option<(&PreparedSemanticAdmission, &Document)>,
    ) -> super::OperationResult<CompiledTransaction> {
        let state = self.derived_state.as_ref().ok_or_else(|| {
            super::OperationError::engine_invariant_failed(
                transaction.request_id,
                None,
                "ready Yrs engine has no derived state",
            )
        })?;
        let txn = self.doc.transact();
        let fragment = txn
            .get_xml_fragment(self.fragment_name.as_str())
            .ok_or_else(|| {
                super::OperationError::engine_invariant_failed(
                    transaction.request_id,
                    None,
                    "ready Yrs document fragment is missing",
                )
            })?;
        let installed_authority =
            super::prepared_admission::InstalledDerivedStateAuthority::new(state);
        self.compile_typed_transaction_with_read_view(
            transaction,
            prepared_semantics,
            &installed_authority,
            &txn,
            &fragment,
        )
    }

    fn compile_typed_transaction_with_read_view<T: ReadTxn>(
        &self,
        transaction: super::TypedTransaction,
        prepared_semantics: Option<(&PreparedSemanticAdmission, &Document)>,
        authority: &dyn super::prepared_admission::DerivedStateAuthority,
        txn: &T,
        fragment: &XmlFragmentRef,
    ) -> super::OperationResult<CompiledTransaction> {
        let state = authority.installed();
        let cached = state.compilation_view();
        let document = cached.document;
        let current_selection = cached.selection;
        let current_relative_selection = self.relative_selection().cloned();
        let compilation_context = CompilationContext {
            document,
            selection: Some(current_selection),
            schema: &self.schema,
            resource_limits: &self.resource_limits,
            editing_limits: &self.editing_limits,
            document_revision: self.revision,
            max_length: self.max_length,
        };
        let stored_marks_context = StoredMarksCompilationContext {
            stored_marks: state.stored_marks.as_deref(),
            resolved_selection: &state.resolved_selection,
            relative_selection: &state.relative_selection,
        };
        let engine_view = EngineCompilationView {
            cached,
            authority,
            state_revision: self.state_revision,
            schema_fingerprint: &self.schema_fingerprint,
            yrs_state_epoch: self.yrs_state_epoch,
        };
        let mut compiled = if let Some((semantic_admission, expected_preview)) = prepared_semantics
        {
            compile_prepared_transaction_with_yrs_and_stored_marks(
                compilation_context,
                transaction,
                txn,
                fragment,
                stored_marks_context,
                PreparedSemanticContext {
                    admission: semantic_admission,
                    expected_preview,
                    yrs_state_epoch: self.yrs_state_epoch,
                    state_revision: self.state_revision,
                    schema_fingerprint: &self.schema_fingerprint,
                },
                engine_view,
            )?
        } else {
            compile_transaction_with_yrs_and_stored_marks(
                compilation_context,
                transaction,
                txn,
                fragment,
                stored_marks_context,
                engine_view,
            )?
        };
        if let (
            Some(selection),
            Some(relative),
            SelectionPlan::Mapped(_),
            RelativeSelectionPlan::PreserveWithFallback(fallback),
        ) = (
            Some(current_selection),
            current_relative_selection.as_ref(),
            &compiled.selection_plan,
            &mut compiled.relative_selection_plan,
        ) {
            *fallback = affinity_aware_mapped_selection(
                selection,
                relative,
                &compiled.composed_map,
                &compiled.preview,
                &self.schema,
                compiled
                    .preview_derivations
                    .as_ref()
                    .map(|derivations| &derivations.position_map),
            );
        }
        if let RelativeSelectionPlan::Precomputed { relative, fallback } =
            &compiled.relative_selection_plan
        {
            if compiled.preview != *document
                && selection_requires_fallback_proof(
                    &compiled.mutation_plan,
                    txn,
                    fragment,
                    relative,
                )
            {
                let proof_source = ValidatedImportDocument {
                    document: compiled.preview.clone(),
                    canonical_artifact: compiled.canonical_artifact.as_ref().cloned().ok_or_else(
                        || {
                            super::OperationError::engine_invariant_failed(
                                compiled.request_id,
                                None,
                                "changed explicit selection preview has no canonical JSON",
                            )
                        },
                    )?,
                    validation: RootBoundValidationReport {
                        source_root: compiled.preview.root().clone(),
                        report: DocumentValidator::validate_report(
                            &compiled.preview,
                            &self.schema,
                            &self.resource_limits,
                        )
                        .map_err(|error| {
                            super::OperationError::engine_invariant_failed(
                                compiled.request_id,
                                None,
                                format!("selection proof preview is invalid: {error}"),
                            )
                        })?,
                    },
                    carry_import_encoded_state_receipt: false,
                };
                let proof = self
                    .build_candidate_from_document(proof_source, compiled.origin)
                    .map_err(|error| {
                        super::OperationError::engine_invariant_failed(
                            compiled.request_id,
                            None,
                            format!("cannot prove committed selection representation: {error}"),
                        )
                    })?;
                let proof_txn = proof.doc.transact();
                let proof_fragment = proof_txn
                    .get_xml_fragment(self.fragment_name.as_str())
                    .ok_or_else(|| {
                        super::OperationError::engine_invariant_failed(
                            compiled.request_id,
                            None,
                            "selection proof candidate fragment is missing",
                        )
                    })?;
                if !required_fallbacks_are_representable(
                    FallbackProofContext {
                        plan: &compiled.mutation_plan,
                        current_txn: txn,
                        current_fragment: fragment,
                        proof_txn: &proof_txn,
                        proof_fragment: &proof_fragment,
                        schema: &self.schema,
                    },
                    fallback,
                    relative,
                ) {
                    return Err(super::OperationError::selection_position_invalid(
                        compiled.request_id,
                        "selection",
                        "mapped selection cannot preserve the requested Yrs affinity",
                    ));
                }
            }
        }
        compiled.yrs_state_epoch = self.yrs_state_epoch;
        Ok(compiled)
    }

    fn with_compiled_base_authority<R>(
        &self,
        request_id: u64,
        context: Option<&super::prepared_admission::PreparedMutationContext>,
        use_authority: impl FnOnce(
            &dyn super::prepared_admission::DerivedStateAuthority,
            &yrs::Transaction<'_>,
            &XmlFragmentRef,
        ) -> super::OperationResult<R>,
    ) -> super::OperationResult<R> {
        let state = self
            .derived_state
            .as_ref()
            .ok_or_else(|| super::OperationError::engine_not_ready(request_id))?;
        #[cfg(test)]
        record_compiled_commit_live_view_for_test();
        let txn = self.doc.transact();
        let fragment = txn
            .get_xml_fragment(self.fragment_name.as_str())
            .ok_or_else(|| {
                super::OperationError::engine_invariant_failed(
                    request_id,
                    None,
                    "compiled transaction lost its live Yrs fragment",
                )
            })?;
        if let Some(context) = context {
            #[cfg(test)]
            record_compiled_commit_authority_validation_for_test();
            let authority =
                context.authority(super::prepared_admission::LiveMutationAuthorityContext {
                    request_id,
                    installed: state,
                    txn: &txn,
                    fragment: &fragment,
                    fragment_name: &self.fragment_name,
                    schema_fingerprint: &self.schema_fingerprint,
                    resource_limits: &self.resource_limits,
                    editing_limits: &self.editing_limits,
                    max_length: self.max_length,
                    document_revision: self.revision,
                    state_revision: self.state_revision,
                    yrs_state_epoch: self.yrs_state_epoch,
                })?;
            use_authority(&authority, &txn, &fragment)
        } else {
            let authority = super::prepared_admission::InstalledDerivedStateAuthority::new(state);
            use_authority(&authority, &txn, &fragment)
        }
    }

    fn apply_prepared_command_transaction(
        &mut self,
        transaction: super::TypedTransaction,
        proof: super::commands::PreparedCommandProof,
        with_result: bool,
        outbound: &mut OutboundUpdateSink<'_>,
    ) -> super::OperationResult<(
        super::TransactionCommit,
        Option<super::TypedTransactionResult>,
    )> {
        let request_id = transaction.request_id;
        let super::commands::PreparedCommandProof {
            document,
            selection,
            execution_admission,
        } = proof;
        execution_admission.pre_admit_seed_independent(
            &transaction,
            &document,
            &self.editing_limits,
        )?;
        let prepare_history_before_context = self
            .derived_state
            .as_ref()
            .is_some_and(|state| state.mutation_lookup_seed.is_unavailable());
        let prepared_history = if prepare_history_before_context {
            self.prepare_execution_command_history_admission(&execution_admission)?
        } else {
            None
        };
        let requires_identity = execution_admission.requires_materialized_identity();
        let prepared_execution = super::prepared_admission::PreparedExecutionAdmission::new(
            execution_admission,
            prepared_history,
        );
        let mut context = self.prepare_mutation_lookup_seed(request_id)?;
        if requires_identity {
            self.prepare_mutation_identity(&mut context)?;
        }

        let (execution_admission, prepared_history) = prepared_execution.into_parts();
        let compiled = {
            let state = self
                .derived_state
                .as_ref()
                .ok_or_else(|| super::OperationError::engine_not_ready(request_id))?;
            let txn = self.doc.transact();
            let fragment = txn
                .get_xml_fragment(self.fragment_name.as_str())
                .ok_or_else(|| {
                    super::OperationError::engine_invariant_failed(
                        request_id,
                        None,
                        "ready engine lost its prepared-command fragment",
                    )
                })?;
            let authority =
                context.authority(super::prepared_admission::LiveMutationAuthorityContext {
                    request_id,
                    installed: state,
                    txn: &txn,
                    fragment: &fragment,
                    fragment_name: &self.fragment_name,
                    schema_fingerprint: &self.schema_fingerprint,
                    resource_limits: &self.resource_limits,
                    editing_limits: &self.editing_limits,
                    max_length: self.max_length,
                    document_revision: self.revision,
                    state_revision: self.state_revision,
                    yrs_state_epoch: self.yrs_state_epoch,
                })?;
            let semantic_admission = match execution_admission {
                super::prepared_admission::ExecutionSemanticAdmission::Eager(admission) => {
                    admission
                }
                super::prepared_admission::ExecutionSemanticAdmission::Deferred(deferred) => {
                    super::compiler::finalize_deferred_admission(
                        &authority,
                        deferred,
                        super::compiler::PreparedSemanticLiveContext {
                            transaction: &transaction,
                            expected_preview: &document,
                            canonical_schema: &self.canonical_schema,
                        },
                    )?
                }
            };
            let compiled = self.compile_finalized_prepared_typed_transaction_with_read_view(
                transaction,
                &semantic_admission,
                &document,
                &selection,
                prepared_history
                    .as_ref()
                    .map(|history| &history.candidate_derivations),
                &authority,
                &txn,
                &fragment,
            )?;
            if !self.compiled_command_matches_proof(&compiled, &document, &selection)? {
                return Err(super::OperationError::engine_invariant_failed(
                    request_id,
                    None,
                    "prepared command diverged during Yrs compilation",
                ));
            }
            compiled
        };
        self.apply_compiled_transaction_with_history_and_context(
            compiled,
            with_result,
            prepared_history,
            Some(context),
            outbound,
        )
    }

    fn apply_typed_transaction_with_staged_context(
        &mut self,
        transaction: super::TypedTransaction,
        with_result: bool,
        outbound: &mut OutboundUpdateSink<'_>,
    ) -> super::OperationResult<(
        super::TransactionCommit,
        Option<super::TypedTransactionResult>,
    )> {
        if transaction.operations.is_empty() {
            if transaction.history_policy == super::HistoryPolicy::Skip {
                return self.apply_empty_skip_transaction(transaction, with_result);
            }
            let compiled = self.compile_typed_transaction(transaction)?;
            return self.apply_compiled_transaction_with_history(
                compiled,
                with_result,
                None,
                outbound,
            );
        }
        let request_id = transaction.request_id;
        let requires_identity = matches!(
            transaction.operations.as_slice(),
            [super::TypedOperation::InsertText { .. }]
        );
        let mut context = self.prepare_mutation_lookup_seed(request_id)?;
        if requires_identity {
            self.prepare_mutation_identity(&mut context)?;
        }
        let compiled = self.with_compiled_base_authority(
            request_id,
            Some(&context),
            |authority, txn, fragment| {
                self.compile_typed_transaction_with_read_view(
                    transaction,
                    None,
                    authority,
                    txn,
                    fragment,
                )
            },
        )?;
        self.apply_compiled_transaction_with_context(compiled, context, with_result, outbound)
    }

    #[allow(dead_code)]
    pub fn apply_typed_transaction(
        &mut self,
        transaction: super::TypedTransaction,
    ) -> super::OperationResult<super::TransactionCommit> {
        let (commit, _) = self.apply_typed_transaction_with_staged_context(
            transaction,
            false,
            &mut OutboundUpdateSink::detached(),
        )?;
        Ok(commit)
    }

    /// Production surface: one typed transaction with an optionally attached
    /// collaboration outbox for outbound update capture. Returns the commit
    /// and, when `with_result` is set, the full typed result envelope.
    pub(crate) fn apply_typed_transaction_with_outbox(
        &mut self,
        transaction: super::TypedTransaction,
        with_result: bool,
        outbox: Option<&mut crate::collaboration_runtime::CollaborationOutbox>,
    ) -> super::OperationResult<(
        super::TransactionCommit,
        Option<super::TypedTransactionResult>,
    )> {
        self.apply_typed_transaction_with_staged_context(
            transaction,
            with_result,
            &mut OutboundUpdateSink::from_optional_outbox(outbox),
        )
    }

    fn apply_empty_skip_transaction(
        &mut self,
        transaction: super::TypedTransaction,
        with_result: bool,
    ) -> super::OperationResult<(
        super::TransactionCommit,
        Option<super::TypedTransactionResult>,
    )> {
        debug_assert!(transaction.operations.is_empty());
        debug_assert_eq!(transaction.history_policy, super::HistoryPolicy::Skip);
        let request_id = transaction.request_id;
        let current = self
            .derived_state
            .as_ref()
            .ok_or_else(|| super::OperationError::engine_not_ready(request_id))?;
        let context = CompilationContext {
            document: &current.document,
            selection: None,
            schema: &self.schema,
            resource_limits: &self.resource_limits,
            editing_limits: &self.editing_limits,
            document_revision: self.revision,
            max_length: self.max_length,
        };
        #[cfg(test)]
        super::compiler::check_atomic_failpoint(
            request_id,
            super::compiler::AtomicFailpoint::EnvelopeAdmission,
        )?;
        let admitted_input_bytes =
            super::compiler::admit_transaction_envelope(context, &transaction)?;
        #[cfg(test)]
        super::compiler::check_atomic_failpoint(
            request_id,
            super::compiler::AtomicFailpoint::SemanticCompilation,
        )?;

        let txn = self.doc.transact();
        super::compiler::admit_yrs_scan_work(
            request_id,
            admitted_input_bytes,
            current.document_text_bytes,
            &txn,
            &self.resource_limits,
        )?;
        let needs_rendered_text = match &transaction.selection_intent {
            super::SelectionIntent::Set(super::SelectionInput::Text { anchor, head }) => {
                anchor.kind == super::EditorOffsetKind::Utf16
                    || head.kind == super::EditorOffsetKind::Utf16
            }
            super::SelectionIntent::Set(super::SelectionInput::Node { at }) => {
                at.kind == super::EditorOffsetKind::Utf16
            }
            _ => false,
        };
        let rendered_text = if needs_rendered_text {
            current.rendered_text.as_str()
        } else {
            ""
        };
        let fragment = txn
            .get_xml_fragment(self.fragment_name.as_str())
            .ok_or_else(|| {
                super::OperationError::engine_invariant_failed(
                    request_id,
                    None,
                    "ready Yrs document fragment is missing",
                )
            })?;
        let resolve_point = |field: &'static str,
                             point: super::RevisionedPosition|
         -> super::OperationResult<u32> {
            super::position::editor_offset_to_doc_pos(
                point.offset,
                point.kind,
                rendered_text,
                &current.position_map,
                &current.document,
            )
            .ok_or_else(|| {
                super::OperationError::selection_position_invalid(
                    request_id,
                    field,
                    format!("{field} is outside the base document"),
                )
            })
        };
        let relative_point = |field: &'static str,
                              point: super::RevisionedPosition|
         -> super::OperationResult<super::RelativePoint> {
            let document_position = resolve_point(field, point)?;
            super::position::doc_pos_to_relative_point(
                &txn,
                &fragment,
                document_position,
                point.affinity,
                &self.schema,
            )
            .ok_or_else(|| {
                super::OperationError::selection_position_invalid(
                    request_id,
                    field,
                    "selection cannot be represented with the requested Yrs affinity",
                )
            })
        };
        let mut prepared_next_selection = None;
        let next_relative = match &transaction.selection_intent {
            super::SelectionIntent::Preserve | super::SelectionIntent::UseOperationResult => {
                current.relative_selection.clone()
            }
            super::SelectionIntent::Set(super::SelectionInput::Text { anchor, head }) => {
                let anchor_document = resolve_point("selection.anchor", *anchor)?;
                let head_document = if anchor == head {
                    anchor_document
                } else {
                    resolve_point("selection.head", *head)?
                };
                let normalized = Selection::text(anchor_document, head_document)
                    .normalized(&current.document, &current.position_map);
                debug_assert!(matches!(normalized, Selection::Text { .. }));
                let prepared_collapsed = if anchor == head {
                    let Selection::Text {
                        anchor: normalized_anchor,
                        head: normalized_head,
                    } = normalized
                    else {
                        unreachable!("text selection normalized to a non-text selection")
                    };
                    (normalized_anchor == anchor_document
                        && normalized_head == head_document
                        && normalized_anchor == normalized_head)
                        .then(|| {
                            let relative = super::position::admitted_doc_pos_to_relative_point(
                                &txn,
                                &fragment,
                                normalized_anchor,
                                anchor.affinity,
                                &self.schema,
                            )?;
                            let scalar = current
                                .position_map
                                .doc_to_scalar(normalized_anchor, &current.document);
                            let utf16 = super::position::scalar_offset_to_utf16(
                                &current.rendered_text,
                                scalar,
                            )?;
                            let resolved = super::ResolvedPoint {
                                document: normalized_anchor,
                                scalar,
                                utf16,
                            };
                            Some((relative, resolved))
                        })
                        .flatten()
                } else {
                    None
                };
                if let Some((point, resolved)) = prepared_collapsed {
                    prepared_next_selection = Some(super::ResolvedSelection::Text {
                        anchor: resolved,
                        head: resolved,
                    });
                    super::RelativeSelection::Text {
                        anchor: point.clone(),
                        head: point,
                    }
                } else {
                    super::RelativeSelection::Text {
                        anchor: relative_point("selection.anchor", *anchor)?,
                        head: relative_point("selection.head", *head)?,
                    }
                }
            }
            super::SelectionIntent::Set(super::SelectionInput::Node { at }) => {
                let document_position = resolve_point("selection.at", *at)?;
                let normalized = Selection::node(document_position)
                    .normalized(&current.document, &current.position_map);
                let Selection::Node { pos } = normalized else {
                    return Err(super::OperationError::engine_invariant_failed(
                        request_id,
                        None,
                        "node selection did not compile to a node selection",
                    ));
                };
                if !selectable_void_at(current.document.root(), pos, 0, &self.schema) {
                    return Err(super::OperationError::selection_position_invalid(
                        request_id,
                        "selection.at",
                        "node selection must target a selectable void or atom node",
                    ));
                }
                super::RelativeSelection::Node {
                    point: relative_point("selection.at", *at)?,
                }
            }
            super::SelectionIntent::Set(super::SelectionInput::All) => {
                super::RelativeSelection::All
            }
        };
        let next_selection = match prepared_next_selection {
            Some(selection) => selection,
            None => current
                .resolve_relative_selection(&next_relative, &txn, &fragment, &self.schema)
                .ok_or_else(|| {
                    super::OperationError::selection_position_invalid(
                        request_id,
                        "selection",
                        "selection cannot be represented in the Yrs document",
                    )
                })?,
        };
        drop(txn);
        let next_stored_marks = stored_marks_after_selection_change(
            current.stored_marks.as_deref(),
            &current.resolved_selection,
            &next_selection,
            &current.document,
            &self.schema,
        );
        let changed = next_relative != current.relative_selection
            || next_selection != current.resolved_selection
            || next_stored_marks != current.stored_marks;
        let next_state_revision = if changed {
            checked_operation_increment(request_id, self.state_revision, "stateRevision")?
        } else {
            self.state_revision
        };
        let result = with_result
            .then(|| {
                self.prepare_empty_skip_result(
                    request_id,
                    transaction.origin,
                    &next_selection,
                    next_stored_marks.as_deref(),
                    changed,
                    next_state_revision,
                )
            })
            .transpose()?;

        if changed {
            let current = self.derived_state.as_mut().ok_or_else(|| {
                super::OperationError::engine_invariant_failed(
                    request_id,
                    None,
                    "ready Yrs engine lost derived state during selection admission",
                )
            })?;
            current.update_selection_state(
                next_relative,
                next_selection,
                next_stored_marks,
                next_state_revision,
            );
            self.state_revision = next_state_revision;
            self.last_committed_origin = Some(transaction.origin);
        }
        let commit = super::TransactionCommit {
            request_id,
            changed,
            document_revision: self.revision,
            state_revision: self.state_revision,
            origin: transaction.origin,
        };
        Ok((commit, result))
    }

    fn prepare_empty_skip_result(
        &self,
        request_id: u64,
        origin: TransactionOrigin,
        selection: &super::ResolvedSelection,
        stored_marks: Option<&[crate::model::Mark]>,
        changed: bool,
        state_revision: u64,
    ) -> super::OperationResult<super::TypedTransactionResult> {
        let current = self
            .derived_state
            .as_ref()
            .ok_or_else(|| super::OperationError::engine_not_ready(request_id))?;
        let legacy_selection = super::derived_state::resolved_to_legacy(selection);
        let commands = crate::editor_state::command_applicability_with_known_node_count(
            &current.document,
            &self.schema,
            &legacy_selection,
            &self.resource_limits,
            current.document_node_count,
        );
        let active_state = crate::editor_state::active_state(
            &current.document,
            &self.schema,
            &legacy_selection,
            stored_marks,
            commands,
            &self.resource_limits,
        );
        let result = super::TypedTransactionResult {
            request_id,
            origin,
            changed,
            document_revision: self.revision,
            state_revision,
            selection: selection.clone(),
            active_state,
            history_state: crate::editor_state::HistoryState {
                can_undo: self.can_undo(),
                can_redo: self.can_redo(),
            },
            render_update: super::RenderUpdate::None,
        };
        self.admit_typed_result(request_id, &result)?;
        Ok(result)
    }

    fn prepare_typed_result(
        &self,
        compiled: &CompiledTransaction,
        render_update: super::RenderUpdate,
        commit_authority: &CompiledCommitAuthority<'_, '_>,
    ) -> super::OperationResult<(
        super::TypedTransactionResult,
        Option<Arc<super::derived_state::CachedActiveState>>,
    )> {
        let current = self
            .derived_state
            .as_ref()
            .ok_or_else(|| super::OperationError::engine_not_ready(compiled.request_id))?;
        let selection = match &compiled.selection_plan {
            SelectionPlan::Preserve => current.resolved_selection.clone(),
            SelectionPlan::Explicit(selection) | SelectionPlan::Mapped(selection) => {
                let (position_map, rendered_text) = compiled
                    .preview_derivations
                    .as_ref()
                    .map(|derivations| {
                        (
                            &derivations.position_map,
                            derivations.rendered_text.as_str(),
                        )
                    })
                    .unwrap_or((&current.position_map, current.rendered_text.as_str()));
                super::derived_state::resolved_from_legacy_with_view(
                    &compiled.preview,
                    selection,
                    &self.schema,
                    position_map,
                    rendered_text,
                )
                .ok_or_else(|| {
                    super::OperationError::engine_invariant_failed(
                        compiled.request_id,
                        None,
                        "compiled result selection cannot be resolved",
                    )
                })?
            }
        };
        let StoredMarksPlan::Set(stored_marks) = &compiled.stored_marks_plan else {
            return Err(super::OperationError::engine_invariant_failed(
                compiled.request_id,
                None,
                "compiled result stored-mark plan is not sealed",
            ));
        };
        let legacy_selection = super::derived_state::resolved_to_legacy(&selection);
        let document_node_count = compiled
            .preview_derivations
            .as_ref()
            .map(|derivations| derivations.document_node_count)
            .unwrap_or(current.document_node_count);
        let generic_active_state = || {
            super::derived_state::record_active_state_generic_build();
            let commands = crate::editor_state::command_applicability_with_known_node_count(
                &compiled.preview,
                &self.schema,
                &legacy_selection,
                &self.resource_limits,
                document_node_count,
            );
            crate::editor_state::active_state(
                &compiled.preview,
                &self.schema,
                &legacy_selection,
                stored_marks.as_deref(),
                commands,
                &self.resource_limits,
            )
        };
        let (active_state, prepared_active_cache) =
            if let Some(transition) = &compiled.prepared_active_state_transition {
                super::derived_state::record_active_state_cache_attempt();
                let structural = compiled.localized_insert_admission.as_ref().map(
                    super::derived_state::LocalizedInsertAdmission::active_state_structural_seal,
                );
                let validated = if let Some(structural) = structural.as_ref() {
                    current.validate_active_state_transition(
                        commit_authority.derived(),
                        transition,
                        structural,
                        &compiled.preview,
                        &selection,
                        stored_marks.as_deref(),
                        &self.resource_limits,
                        &self.editing_limits,
                        self.max_length,
                        self.yrs_state_epoch,
                    )
                } else {
                    None
                };
                match validated {
                    Some(cached) => {
                        super::derived_state::record_active_state_candidate_build();
                        let warm_cached = cached.filter(|_| {
                            !super::derived_state::active_state_cache_hit_fallback_forced()
                        });
                        let was_warm = warm_cached.is_some();
                        let cached = if let Some(cached) = warm_cached {
                            cached
                        } else {
                            super::derived_state::record_active_state_cache_fallback();
                            let generic = generic_active_state();
                            match super::derived_state::CachedActiveState::try_new(
                                generic,
                                &self.resource_limits,
                                &self.editing_limits,
                            ) {
                                Ok(cached) => cached,
                                Err(generic) => {
                                    let result = super::TypedTransactionResult {
                                        request_id: compiled.request_id,
                                        origin: compiled.origin,
                                        changed: current.document != compiled.preview,
                                        document_revision: self.revision,
                                        state_revision: self.state_revision,
                                        selection,
                                        active_state: generic,
                                        history_state: crate::editor_state::HistoryState {
                                            can_undo: self.can_undo(),
                                            can_redo: self.can_redo(),
                                        },
                                        render_update,
                                    };
                                    self.admit_typed_result(compiled.request_id, &result)?;
                                    return Ok((result, None));
                                }
                            }
                        };
                        #[cfg(test)]
                        debug_assert_eq!(
                            cached.value(),
                            &crate::editor_state::active_state_for_debug_invariant(
                                &compiled.preview,
                                &self.schema,
                                &legacy_selection,
                                stored_marks.as_deref(),
                                &self.resource_limits,
                                document_node_count,
                            )
                        );
                        if let Some(active_state) =
                            cached.clone_public(&self.resource_limits, &self.editing_limits)
                        {
                            if was_warm {
                                super::derived_state::record_active_state_cache_hit();
                            }
                            (active_state, Some(cached))
                        } else {
                            if was_warm {
                                super::derived_state::record_active_state_cache_fallback();
                                (generic_active_state(), None)
                            } else {
                                let generic =
                                    super::derived_state::CachedActiveState::try_into_value(cached)
                                        .unwrap_or_else(|cached| cached.value().clone());
                                (generic, None)
                            }
                        }
                    }
                    None => {
                        super::derived_state::record_active_state_cache_fallback();
                        (generic_active_state(), None)
                    }
                }
            } else {
                // Non-eligible result paths retain the existing generic behavior
                // and are outside the active-state cache lifecycle counters.
                let commands = crate::editor_state::command_applicability_with_known_node_count(
                    &compiled.preview,
                    &self.schema,
                    &legacy_selection,
                    &self.resource_limits,
                    document_node_count,
                );
                (
                    crate::editor_state::active_state(
                        &compiled.preview,
                        &self.schema,
                        &legacy_selection,
                        stored_marks.as_deref(),
                        commands,
                        &self.resource_limits,
                    ),
                    None,
                )
            };
        let result = super::TypedTransactionResult {
            request_id: compiled.request_id,
            origin: compiled.origin,
            changed: current.document != compiled.preview,
            document_revision: self.revision,
            state_revision: self.state_revision,
            selection,
            active_state,
            history_state: crate::editor_state::HistoryState {
                can_undo: self.can_undo(),
                can_redo: self.can_redo(),
            },
            render_update,
        };
        self.admit_typed_result(compiled.request_id, &result)?;
        Ok((result, prepared_active_cache))
    }

    fn admit_typed_result(
        &self,
        request_id: u64,
        result: &super::TypedTransactionResult,
    ) -> super::OperationResult<()> {
        let actual = result.derived_output_bytes();
        if actual > self.editing_limits.max_derived_output_bytes {
            return Err(super::OperationError::document_limit_exceeded(
                request_id,
                None,
                "maxDerivedOutputBytes",
                u64::try_from(self.editing_limits.max_derived_output_bytes).unwrap_or(u64::MAX),
                u64::try_from(actual).unwrap_or(u64::MAX),
            ));
        }
        let render_elements = match &result.render_update {
            super::RenderUpdate::None => 0,
            super::RenderUpdate::Patch(patch) => patch.blocks.iter().map(Vec::len).sum(),
            super::RenderUpdate::Full(blocks) => blocks.iter().map(Vec::len).sum(),
        };
        let render_element_limit = self.resource_limits.max_document_nodes.saturating_mul(3);
        if render_elements > render_element_limit {
            return Err(super::OperationError::document_limit_exceeded(
                request_id,
                None,
                "maxDocumentNodes",
                u64::try_from(render_element_limit).unwrap_or(u64::MAX),
                u64::try_from(render_elements).unwrap_or(u64::MAX),
            ));
        }
        let schema_marks = self.schema.all_marks().count();
        let schema_nodes = self.schema.all_nodes().count();
        let active_is_bounded = result.active_state.marks.len() <= schema_marks
            && result.active_state.mark_attrs.len() <= schema_marks
            && result.active_state.allowed_marks.len() <= schema_marks
            && result.active_state.nodes.len()
                <= self.resource_limits.max_document_depth.saturating_add(1)
            && result.active_state.insertable_nodes.len() <= schema_nodes
            && result.active_state.commands.len() <= 16;
        if !active_is_bounded {
            return Err(super::OperationError::document_limit_exceeded(
                request_id,
                None,
                "maxSchemaNodes",
                u64::try_from(schema_nodes.max(schema_marks)).unwrap_or(u64::MAX),
                u64::MAX,
            ));
        }
        Ok(())
    }

    #[allow(dead_code)]
    pub fn apply_typed_transaction_with_result(
        &mut self,
        transaction: super::TypedTransaction,
    ) -> super::OperationResult<super::TypedTransactionResult> {
        let request_id = transaction.request_id;
        let (_, result) = self.apply_typed_transaction_with_staged_context(
            transaction,
            true,
            &mut OutboundUpdateSink::detached(),
        )?;
        result.ok_or_else(|| {
            super::OperationError::engine_invariant_failed(
                request_id,
                None,
                "rich typed transaction produced no result envelope",
            )
        })
    }

    #[cfg(test)]
    fn apply_compiled_transaction(
        &mut self,
        compiled: CompiledTransaction,
        with_result: bool,
    ) -> super::OperationResult<(
        super::TransactionCommit,
        Option<super::TypedTransactionResult>,
    )> {
        self.apply_compiled_transaction_with_history(
            compiled,
            with_result,
            None,
            &mut OutboundUpdateSink::detached(),
        )
    }

    fn apply_compiled_transaction_with_context(
        &mut self,
        compiled: CompiledTransaction,
        context: super::prepared_admission::PreparedMutationContext,
        with_result: bool,
        outbound: &mut OutboundUpdateSink<'_>,
    ) -> super::OperationResult<(
        super::TransactionCommit,
        Option<super::TypedTransactionResult>,
    )> {
        self.apply_compiled_transaction_with_history_and_context(
            compiled,
            with_result,
            None,
            Some(context),
            outbound,
        )
    }

    fn apply_compiled_transaction_with_history(
        &mut self,
        compiled: CompiledTransaction,
        with_result: bool,
        prepared_history: Option<super::prepared_admission::PreparedCommandHistoryAdmission>,
        outbound: &mut OutboundUpdateSink<'_>,
    ) -> super::OperationResult<(
        super::TransactionCommit,
        Option<super::TypedTransactionResult>,
    )> {
        self.apply_compiled_transaction_with_history_and_context(
            compiled,
            with_result,
            prepared_history,
            None,
            outbound,
        )
    }

    fn apply_compiled_transaction_with_history_and_context(
        &mut self,
        mut compiled: CompiledTransaction,
        with_result: bool,
        prepared_history: Option<super::prepared_admission::PreparedCommandHistoryAdmission>,
        prepared_context: Option<super::prepared_admission::PreparedMutationContext>,
        outbound: &mut OutboundUpdateSink<'_>,
    ) -> super::OperationResult<(
        super::TransactionCommit,
        Option<super::TypedTransactionResult>,
    )> {
        #[cfg(test)]
        begin_compiled_commit_preparation_for_test();
        // A compiled plan owns Yrs handles after its original read transaction
        // closes. Reject a stale plan in O(1) before no-op classification or
        // any state-vector/snapshot traversal.
        if compiled.yrs_state_epoch != self.yrs_state_epoch
            || compiled.base_state_revision != self.state_revision
        {
            return Err(super::OperationError::engine_invariant_failed(
                compiled.request_id,
                None,
                "compiled Yrs transaction is stale",
            ));
        }
        let installed = self
            .derived_state
            .as_ref()
            .ok_or_else(|| super::OperationError::engine_not_ready(compiled.request_id))?;
        let authority_doc = self.doc.clone();
        #[cfg(test)]
        record_compiled_commit_live_view_for_test();
        let authority_txn = authority_doc.transact();
        let authority_fragment = authority_txn
            .get_xml_fragment(self.fragment_name.as_str())
            .ok_or_else(|| {
                super::OperationError::engine_invariant_failed(
                    compiled.request_id,
                    None,
                    "compiled transaction lost its live Yrs fragment",
                )
            })?;
        let derived = if let Some(context) = prepared_context.as_ref() {
            #[cfg(test)]
            record_compiled_commit_authority_validation_for_test();
            CompiledCommitDerivedAuthority::Staged(context.authority(
                super::prepared_admission::LiveMutationAuthorityContext {
                    request_id: compiled.request_id,
                    installed,
                    txn: &authority_txn,
                    fragment: &authority_fragment,
                    fragment_name: &self.fragment_name,
                    schema_fingerprint: &self.schema_fingerprint,
                    resource_limits: &self.resource_limits,
                    editing_limits: &self.editing_limits,
                    max_length: self.max_length,
                    document_revision: self.revision,
                    state_revision: self.state_revision,
                    yrs_state_epoch: self.yrs_state_epoch,
                },
            )?)
        } else {
            CompiledCommitDerivedAuthority::Installed(
                super::prepared_admission::InstalledDerivedStateAuthority::new(installed),
            )
        };
        let commit_authority = CompiledCommitAuthority {
            derived,
            txn: &authority_txn,
            fragment: &authority_fragment,
            state_vector: std::cell::OnceCell::new(),
        };
        let preview_is_unchanged = compiled.preview
            == *self
                .document()
                .ok_or_else(|| super::OperationError::engine_not_ready(compiled.request_id))?;
        if preview_is_unchanged != compiled.mutation_plan.is_empty() {
            return Err(super::OperationError::engine_invariant_failed(
                compiled.request_id,
                None,
                "compiled preview and Yrs mutation plan disagree about document changes",
            ));
        }
        let mut prepared_history_limits = None;
        let mut prepared_history_before = None;
        let mut prepared_history_after = None;
        let mut prepared_history_render = None;
        if let Some(admission) = prepared_history {
            if preview_is_unchanged
                || !admission
                    .candidate_render
                    .cache
                    .matches_identity(&compiled.preview, &self.schema_fingerprint)
                || admission.candidate_derivations.rendered_scalars
                    != admission.candidate_derivations.position_map.total_scalars()
            {
                return Err(super::OperationError::engine_invariant_failed(
                    compiled.request_id,
                    None,
                    "prepared command history admission does not match compiled output",
                ));
            }
            compiled.preview_derivations = Some(admission.candidate_derivations);
            prepared_history_limits = Some(admission.limits);
            prepared_history_before = Some(admission.before);
            prepared_history_after = Some(admission.after);
            prepared_history_render = Some(admission.candidate_render);
        }
        let relative_plan_is_sealed = matches!(
            (&compiled.selection_plan, &compiled.relative_selection_plan),
            (SelectionPlan::Preserve, RelativeSelectionPlan::Preserve)
                | (
                    SelectionPlan::Mapped(_),
                    RelativeSelectionPlan::PreserveWithFallback(_)
                )
                | (
                    SelectionPlan::Explicit(_),
                    RelativeSelectionPlan::Precomputed { .. }
                )
                | (
                    SelectionPlan::Explicit(_),
                    RelativeSelectionPlan::OperationResult
                )
        );
        if !relative_plan_is_sealed {
            return Err(super::OperationError::engine_invariant_failed(
                compiled.request_id,
                None,
                "compiled relative selection plan is not sealed",
            ));
        }
        if !matches!(compiled.stored_marks_plan, StoredMarksPlan::Set(_)) {
            return Err(super::OperationError::engine_invariant_failed(
                compiled.request_id,
                None,
                "compiled stored-mark plan is not sealed",
            ));
        }
        let had_active_state_certificate = self
            .derived_state
            .as_ref()
            .is_some_and(super::derived_state::DerivedStateCache::has_active_state_certificate);
        let render_transition = if preview_is_unchanged {
            None
        } else if let Some(transition) = prepared_history_render.take() {
            Some(transition)
        } else {
            let current = self
                .derived_state
                .as_ref()
                .ok_or_else(|| super::OperationError::engine_not_ready(compiled.request_id))?;
            let generic_transition = || {
                current.render_blocks.transition(
                    &current.document,
                    &compiled.preview,
                    &self.schema,
                    &[],
                    &self.resource_limits,
                )
            };
            let transition = if compiled.localized_semantic_used {
                crate::render::incremental::record_localized_render_transition_attempt();
                let specialized =
                    compiled
                        .prepared_derived_evidence
                        .as_ref()
                        .and_then(|evidence| {
                            evidence.prepare_localized_render_transition(
                                current,
                                &compiled.preview,
                                compiled.preview_derivations.as_ref()?,
                                &compiled.affected_top_level_blocks,
                                &self.schema,
                                &self.schema_fingerprint,
                                &self.resource_limits,
                                &self.editing_limits,
                                self.max_length,
                            )
                        });
                match specialized {
                    Some(Ok(transition)) => {
                        crate::render::incremental::record_localized_render_transition_success();
                        Ok(transition)
                    }
                    Some(Err(_)) => {
                        crate::render::incremental::record_localized_render_transition_fallback();
                        generic_transition()
                    }
                    None => {
                        crate::render::incremental::record_localized_render_transition_fallback();
                        generic_transition()
                    }
                }
            } else {
                generic_transition()
            };
            Some(transition.map_err(|error| {
                cached_render_operation_error(compiled.request_id, &self.resource_limits, error)
            })?)
        };
        let render_update = render_transition
            .as_ref()
            .map(|transition| cached_transition_render_update(&transition.update))
            .unwrap_or(super::RenderUpdate::None);
        let prepared_result = with_result
            .then(|| self.prepare_typed_result(&compiled, render_update, &commit_authority))
            .transpose()?;
        let (mut result, prepared_active_cache) = match prepared_result {
            Some((result, cache)) => (Some(result), cache),
            None => (None, None),
        };
        if preview_is_unchanged {
            let boundary_state =
                (compiled.history_policy == super::HistoryPolicy::Boundary).then(|| {
                    if commit_authority.state_vector().is_empty() {
                        Vec::new()
                    } else {
                        commit_authority
                            .txn()
                            .encode_state_as_update_v1(&StateVector::default())
                    }
                });
            let current = self.derived_state.as_ref().ok_or_else(|| {
                super::OperationError::engine_invariant_failed(
                    compiled.request_id,
                    None,
                    "ready Yrs engine has no derived state",
                )
            })?;
            let (next_relative_selection, next_resolved_selection) =
                if matches!(compiled.selection_plan, SelectionPlan::Preserve) {
                    (
                        current.relative_selection.clone(),
                        current.resolved_selection.clone(),
                    )
                } else {
                    let selection = match &compiled.selection_plan {
                        SelectionPlan::Explicit(selection) | SelectionPlan::Mapped(selection) => {
                            selection
                        }
                        SelectionPlan::Preserve => unreachable!(),
                    };
                    let planned_relative_selection = match &compiled.relative_selection_plan {
                        RelativeSelectionPlan::Precomputed { relative, .. } => relative.clone(),
                        RelativeSelectionPlan::OperationResult => operation_result_to_relative(
                            commit_authority.txn(),
                            commit_authority.fragment(),
                            selection,
                            &self.schema,
                        ),
                        RelativeSelectionPlan::Unsealed
                        | RelativeSelectionPlan::Preserve
                        | RelativeSelectionPlan::PreserveWithFallback(_) => {
                            return Err(super::OperationError::engine_invariant_failed(
                            compiled.request_id,
                            None,
                            "selection-only transaction has no materializable relative selection",
                        ));
                        }
                    };
                    let resolved_selection = current
                        .resolve_relative_selection(
                            &planned_relative_selection,
                            commit_authority.txn(),
                            commit_authority.fragment(),
                            &self.schema,
                        )
                        .ok_or_else(|| {
                            super::OperationError::selection_position_invalid(
                                compiled.request_id,
                                "selection",
                                "selection cannot be represented in the Yrs document",
                            )
                        })?;
                    (planned_relative_selection, resolved_selection)
                };
            let StoredMarksPlan::Set(planned_stored_marks) = &compiled.stored_marks_plan else {
                unreachable!()
            };
            let next_stored_marks = planned_stored_marks.clone();
            let state_changed = next_relative_selection != current.relative_selection
                || next_resolved_selection != current.resolved_selection
                || next_stored_marks != current.stored_marks;
            let next_state_revision = state_changed
                .then(|| {
                    checked_operation_increment(
                        compiled.request_id,
                        self.state_revision,
                        "stateRevision",
                    )
                })
                .transpose()?;
            let prepared_boundary = boundary_state
                .map(|encoded| self.history.prepare_boundary(compiled.request_id, encoded))
                .transpose()?;
            if !state_changed {
                drop(commit_authority);
                drop(authority_txn);
                drop(authority_doc);
                if let Some(prepared) = prepared_boundary {
                    self.derived_state
                        .as_mut()
                        .expect("history boundary retains derived state")
                        .clear_active_state_certificate();
                    self.history.commit_boundary(prepared);
                }
                let commit = super::TransactionCommit {
                    request_id: compiled.request_id,
                    changed: false,
                    document_revision: self.revision,
                    state_revision: self.state_revision,
                    origin: compiled.origin,
                };
                if let Some(result) = &mut result {
                    result.changed = false;
                    result.document_revision = self.revision;
                    result.state_revision = self.state_revision;
                    result.history_state = crate::editor_state::HistoryState {
                        can_undo: self.can_undo(),
                        can_redo: self.can_redo(),
                    };
                }
                return Ok((commit, result));
            }
            let next_state_revision =
                next_state_revision.expect("changed state has an admitted next revision");
            debug_assert_eq!(current.document_revision, self.revision);
            let mut next = current.clone_with_fallible_localized_index();
            if had_active_state_certificate {
                super::derived_state::record_active_state_cache_drop();
            }
            next.update_selection_state(
                next_relative_selection,
                next_resolved_selection,
                next_stored_marks,
                next_state_revision,
            );
            drop(commit_authority);
            drop(authority_txn);
            drop(authority_doc);
            self.derived_state = Some(next);
            self.state_revision = next_state_revision;
            self.last_committed_origin = Some(compiled.origin);
            if let Some(prepared) = prepared_boundary {
                self.history.commit_boundary(prepared);
            }
            let commit = super::TransactionCommit {
                request_id: compiled.request_id,
                changed: true,
                document_revision: self.revision,
                state_revision: self.state_revision,
                origin: compiled.origin,
            };
            if let Some(result) = &mut result {
                result.changed = true;
                result.document_revision = self.revision;
                result.state_revision = self.state_revision;
                result.selection = self
                    .derived_state
                    .as_ref()
                    .expect("selection-only result retains derived state")
                    .resolved_selection
                    .clone();
                result.history_state = crate::editor_state::HistoryState {
                    can_undo: self.can_undo(),
                    can_redo: self.can_redo(),
                };
            }
            return Ok((commit, result));
        }

        #[cfg(test)]
        super::compiler::check_atomic_failpoint(
            compiled.request_id,
            super::compiler::AtomicFailpoint::CanonicalOutputAdmission,
        )?;
        let canonical_artifact = compiled.canonical_artifact.take().ok_or_else(|| {
            super::OperationError::engine_invariant_failed(
                compiled.request_id,
                None,
                "changed transaction has no admitted canonical artifact",
            )
        })?;

        // Revalidate sealed signatures against one final stable read view.
        let current_encoded_state = {
            #[cfg(test)]
            super::compiler::check_atomic_failpoint(
                compiled.request_id,
                super::compiler::AtomicFailpoint::FinalPreflight,
            )?;
            match (
                compiled.prepared_selection_state.as_ref(),
                compiled.prepared_selection_mutation_seal.as_ref(),
            ) {
                (Some(prepared), Some(seal)) => {
                    if !seal.matches(&compiled, commit_authority.derived()) {
                        return Err(super::OperationError::engine_invariant_failed(
                            compiled.request_id,
                            None,
                            "prepared selection core seal does not match compiled transaction",
                        ));
                    }
                    let rematerialized =
                        compiled
                            .localized_insert_admission
                            .as_ref()
                            .and_then(|admission| {
                                self.materialize_prewrite_selection_state(
                                    &compiled,
                                    admission,
                                    commit_authority.txn(),
                                )
                            });
                    if rematerialized.as_ref() != Some(prepared) {
                        compiled.prepared_selection_state = None;
                        compiled.prepared_selection_mutation_seal = None;
                        super::derived_state::record_prewrite_selection_proof_fallback();
                    }
                }
                (Some(_), None) | (None, Some(_)) => {
                    return Err(super::OperationError::engine_invariant_failed(
                        compiled.request_id,
                        None,
                        "prepared selection state and core seal lifecycle disagree",
                    ));
                }
                (None, None) => {}
            }
            preflight_mutation_plan(
                compiled.request_id,
                &compiled.mutation_plan,
                commit_authority.txn(),
            )?;
            #[cfg(test)]
            super::compiler::check_atomic_failpoint(
                compiled.request_id,
                super::compiler::AtomicFailpoint::EncodedAdmission,
            )?;
            if commit_authority.state_vector().is_empty() {
                Vec::new()
            } else if let Some(encoded_state) =
                self.prepared_candidate_cache.as_mut().and_then(|cache| {
                    cache.take_matching_encoded_state(
                        &self.doc,
                        commit_authority.fragment(),
                        &compiled.mutation_plan,
                        self.revision,
                        self.yrs_state_epoch,
                        self.resource_limits.max_encoded_state_bytes,
                    )
                })
            {
                #[cfg(test)]
                COMMIT_SEALED_STATE_REUSES.set(COMMIT_SEALED_STATE_REUSES.get().saturating_add(1));
                encoded_state
            } else {
                #[cfg(test)]
                COMMIT_CURRENT_STATE_ENCODINGS
                    .set(COMMIT_CURRENT_STATE_ENCODINGS.get().saturating_add(1));
                commit_authority
                    .txn()
                    .encode_state_as_update_v1(&StateVector::default())
            }
        };
        let admitted_encoded_bytes = current_encoded_state
            .len()
            .checked_add(compiled.encoded_growth_bound)
            .ok_or_else(|| {
                super::OperationError::document_limit_exceeded(
                    compiled.request_id,
                    None,
                    "maxEncodedStateBytes",
                    u64::try_from(self.resource_limits.max_encoded_state_bytes).unwrap_or(u64::MAX),
                    u64::MAX,
                )
            })?;
        if admitted_encoded_bytes > self.resource_limits.max_encoded_state_bytes {
            return Err(super::OperationError::document_limit_exceeded(
                compiled.request_id,
                None,
                "maxEncodedStateBytes",
                u64::try_from(self.resource_limits.max_encoded_state_bytes).unwrap_or(u64::MAX),
                u64::try_from(admitted_encoded_bytes).unwrap_or(u64::MAX),
            ));
        }

        #[cfg(test)]
        super::compiler::check_atomic_failpoint(
            compiled.request_id,
            super::compiler::AtomicFailpoint::RevisionAdmission,
        )?;
        let next_document_revision =
            checked_operation_increment(compiled.request_id, self.revision, "documentRevision")?;
        let next_state_revision =
            checked_operation_increment(compiled.request_id, self.state_revision, "stateRevision")?;
        let next_yrs_state_epoch = checked_operation_increment(
            compiled.request_id,
            self.yrs_state_epoch,
            "yrsStateEpoch",
        )?;
        let prepared_active_state_install = compiled
            .prepared_active_state_transition
            .as_ref()
            .zip(prepared_active_cache)
            .map(|(transition, cached)| {
                super::derived_state::DerivedStateCache::prepare_active_state_install(
                    transition,
                    cached,
                    next_document_revision,
                    next_state_revision,
                    next_yrs_state_epoch,
                )
            });
        #[cfg(test)]
        check_compiled_commit_preparation_stage_for_test(
            compiled.request_id,
            CompiledCommitPreparationStage::DocumentValidation,
        )?;
        let had_prepared_candidate_validation = compiled.prepared_candidate_validation.is_some();
        let mut finalized_derived_evidence = compiled
            .prepared_candidate_validation
            .take()
            .and_then(|validation| {
                validation.finalize(
                    &compiled.preview,
                    &canonical_artifact,
                    compiled.preview_derivations.as_ref()?,
                    &self.schema,
                    &self.resource_limits,
                    &self.editing_limits,
                    self.max_length,
                    &self.schema_fingerprint,
                    &self.canonical_schema,
                    next_document_revision,
                    next_state_revision,
                    next_yrs_state_epoch,
                )
            });
        if had_prepared_candidate_validation && finalized_derived_evidence.is_none() {
            return Err(super::OperationError::engine_invariant_failed(
                compiled.request_id,
                None,
                "prepared candidate validation diverged before durable mutation",
            ));
        }
        if finalized_derived_evidence.is_none() {
            finalized_derived_evidence =
                compiled
                    .prepared_derived_evidence
                    .take()
                    .and_then(|evidence| {
                        evidence.finalize(
                            commit_authority.derived(),
                            &compiled.preview,
                            &canonical_artifact,
                            compiled.preview_derivations.as_ref()?,
                            &render_transition.as_ref()?.cache,
                            &self.resource_limits,
                            &self.editing_limits,
                            self.max_length,
                            &self.schema_fingerprint,
                            next_document_revision,
                            next_state_revision,
                            next_yrs_state_epoch,
                        )
                    });
        }
        if compiled.localized_semantic_used && finalized_derived_evidence.is_none() {
            let authority = commit_authority.derived();
            let derivations = compiled.preview_derivations.as_ref().ok_or_else(|| {
                super::OperationError::engine_invariant_failed(
                    compiled.request_id,
                    None,
                    "localized derived evidence has no compiled derivations",
                )
            })?;
            finalized_derived_evidence = Some(
                authority
                    .installed()
                    .prepare_generic_derived_evidence(
                        compiled.request_id,
                        authority,
                        &compiled.preview,
                        &canonical_artifact,
                        derivations,
                        &self.schema,
                        &self.resource_limits,
                        &self.schema_fingerprint,
                        next_document_revision,
                        next_state_revision,
                        next_yrs_state_epoch,
                    )
                    .ok_or_else(|| {
                        super::OperationError::engine_invariant_failed(
                            compiled.request_id,
                            None,
                            "localized derived evidence could not be rebuilt before mutation",
                        )
                    })?,
            );
        }
        #[cfg(test)]
        super::compiler::check_atomic_failpoint(
            compiled.request_id,
            super::compiler::AtomicFailpoint::DurableMetadataAdmission,
        )?;
        let mut next_durable_client_ids = self.durable_client_ids.clone();
        if compiled.authored_clock_units > 0 {
            next_durable_client_ids.insert(self.client_id());
        }
        let captures_history = compiled.history_policy != super::HistoryPolicy::Skip
            && compiled.history_class != super::compiler::HistoryClass::Skip;
        let (history_before, history_after_template) = if captures_history {
            if let (Some(before), Some(after)) = (
                prepared_history_before.take(),
                prepared_history_after.take(),
            ) {
                let current = self
                    .derived_state
                    .as_ref()
                    .expect("captured history has a current derived state");
                if before.canonical_fingerprint != current.canonical_artifact.sha256()
                    || before.derived_output_bytes != current.canonical_artifact.serialized_len()
                    || after.canonical_fingerprint != canonical_artifact.sha256()
                    || after.derived_output_bytes != canonical_artifact.serialized_len()
                {
                    return Err(super::OperationError::engine_invariant_failed(
                        compiled.request_id,
                        None,
                        "prepared command history snapshots do not match live artifacts",
                    ));
                }
                (Some(before), Some(after))
            } else {
                let StoredMarksPlan::Set(stored_marks) = &compiled.stored_marks_plan else {
                    unreachable!("stored-mark plan was sealed above")
                };
                let before = self
                    .derived_state
                    .as_ref()
                    .expect("captured history has a current derived state");
                // Optional history snapshots are admitted only from the exact
                // precomputed after-map/text derivations that will be installed.
                // If that evidence is unavailable, the normal full restore path
                // remains available and no potentially smaller estimate is used.
                let document_snapshot_retained_bytes = compiled
                    .preview_derivations
                    .as_ref()
                    .and_then(|after_derivations| {
                        history_document_snapshots_fit(
                            before,
                            &compiled.preview,
                            &canonical_artifact,
                            after_derivations,
                            &render_transition.as_ref()?.cache,
                            stored_marks.as_deref(),
                            &self.schema_fingerprint,
                            &self.fragment_name,
                            self.scope.as_ref(),
                            self.editing_limits.max_derived_output_bytes,
                        )
                    });
                let prepared = (
                    Some(history_local_state(
                        before,
                        &self.fragment_name,
                        self.scope.as_ref(),
                        &self.resource_limits,
                        &self.editing_limits,
                        self.max_length,
                        document_snapshot_retained_bytes.map(|bytes| bytes.before),
                    )),
                    Some(history_snapshot_template(
                        &canonical_artifact,
                        stored_marks.as_deref(),
                        &self.fragment_name,
                        document_snapshot_retained_bytes.map(|bytes| bytes.after),
                    )),
                );
                prepared
            }
        } else {
            if prepared_history_limits.is_some()
                || prepared_history_before.is_some()
                || prepared_history_after.is_some()
            {
                return Err(super::OperationError::engine_invariant_failed(
                    compiled.request_id,
                    None,
                    "prepared history admission was supplied for a non-capturing command",
                ));
            }
            (None, None)
        };
        let history_after_metadata_bytes = history_after_template
            .as_ref()
            .map(|template| template.metadata_bytes)
            .unwrap_or(0);

        let outbound_update_upper_bound = compiled.outbound_update_upper_bound();
        let CompiledTransaction {
            request_id,
            origin,
            history_policy,
            history_class,
            undo_units_bound,
            replay_work_units_bound,
            encoded_growth_bound,
            authored_clock_units,
            preview,
            preview_derivations,
            selection_plan,
            relative_selection_plan,
            stored_marks_plan,
            composed_map,
            position_update_mode,
            affected_top_level_blocks,
            mutation_plan,
            mutation_lookup_transition,
            prepared_selection_state,
            ..
        } = compiled;
        let mut prepared_mutation_lookup_seed =
            if let Some(transition) = mutation_lookup_transition.as_ref() {
                #[cfg(test)]
                check_compiled_commit_preparation_stage_for_test(
                    request_id,
                    CompiledCommitPreparationStage::LookupTransition,
                )?;
                Some(self.prepare_mutation_lookup_transition_with_authority(
                    request_id,
                    commit_authority.derived(),
                    transition,
                    commit_authority.txn(),
                    commit_authority.fragment(),
                    &preview,
                    &canonical_artifact,
                    next_yrs_state_epoch,
                    next_document_revision,
                )?)
            } else {
                None
            };
        #[cfg(test)]
        check_compiled_commit_preparation_stage_for_test(
            request_id,
            CompiledCommitPreparationStage::AllocationProbe,
        )?;
        let next_render_blocks = Arc::new(
            render_transition
                .expect("changed transaction has a prepared render transition")
                .cache,
        );
        #[cfg(test)]
        check_compiled_commit_preparation_stage_for_test(
            request_id,
            CompiledCommitPreparationStage::HistoryReservation,
        )?;
        // Preserve the baseline lookup/render error precedence while admitting
        // history before all newly introduced candidate-store work.
        let prepared_history = if captures_history {
            PreparedCompiledHistory::Recorded(self.history.pre_admit_recorded(
                request_id,
                origin,
                history_policy,
                history_class,
                undo_units_bound,
                history_before,
                history_after_metadata_bytes,
                &current_encoded_state,
                encoded_growth_bound,
                prepared_history_limits,
            )?)
        } else {
            PreparedCompiledHistory::Excluded(self.history.pre_admit_compiled_excluded(
                request_id,
                origin,
                replay_work_units_bound,
                &current_encoded_state,
                encoded_growth_bound,
            )?)
        };
        #[cfg(test)]
        check_compiled_commit_preparation_stage_for_test(
            request_id,
            CompiledCommitPreparationStage::OperationPreparation,
        )?;
        let cached_candidate = self.prepared_candidate_cache.take();
        let cached_candidate = cached_candidate
            .and_then(|cache| cache.into_matching_doc(self.revision, self.yrs_state_epoch));
        let (candidate_doc, candidate_state_vector) = if let Some(cached) = cached_candidate {
            #[cfg(test)]
            PREPARED_CANDIDATE_CACHE_HITS
                .set(PREPARED_CANDIDATE_CACHE_HITS.get().saturating_add(1));
            cached
        } else {
            #[cfg(test)]
            PREPARED_CANDIDATE_FULL_BOOTSTRAPS
                .set(PREPARED_CANDIDATE_FULL_BOOTSTRAPS.get().saturating_add(1));
            let candidate_doc = self.new_history_candidate_doc();
            // Root shared types are not encoded until they contain structs.
            // Create the configured fragment explicitly so a valid empty root
            // can rebind its first structural mutation.
            let candidate_fragment =
                candidate_doc.get_or_insert_xml_fragment(self.fragment_name.as_str());
            if AsRef::<Branch>::as_ref(&candidate_fragment).id()
                != AsRef::<Branch>::as_ref(commit_authority.fragment()).id()
            {
                return Err(super::OperationError::engine_invariant_failed(
                    request_id,
                    None,
                    "prepared commit candidate root identity does not match the live store",
                ));
            }
            if !current_encoded_state.is_empty() {
                let current_update =
                    Update::decode_v1(&current_encoded_state).map_err(|error| {
                        super::OperationError::engine_invariant_failed(
                            request_id,
                            None,
                            format!(
                                "admitted current Yrs state cannot seed commit candidate: {error}"
                            ),
                        )
                    })?;
                candidate_doc
                    .transact_mut()
                    .apply_update(current_update)
                    .map_err(|error| {
                        super::OperationError::engine_invariant_failed(
                            request_id,
                            None,
                            format!("admitted current Yrs state cannot initialize commit candidate: {error}"),
                        )
                    })?;
            }
            let candidate_state_vector = candidate_doc.transact().state_vector();
            if &candidate_state_vector != commit_authority.state_vector() {
                return Err(super::OperationError::engine_invariant_failed(
                    request_id,
                    None,
                    "prepared commit candidate state vector does not exactly match the live base",
                ));
            }
            (candidate_doc, candidate_state_vector)
        };
        if candidate_doc.client_id() != self.doc.client_id()
            || candidate_doc.guid() != self.doc.guid()
            || candidate_doc.offset_kind() != self.doc.offset_kind()
            || candidate_doc.skip_gc() != self.doc.skip_gc()
        {
            return Err(super::OperationError::engine_invariant_failed(
                request_id,
                None,
                "prepared commit candidate options do not exactly match the live store",
            ));
        }
        let authored_clock_bound = u32::try_from(authored_clock_units).map_err(|_| {
            super::OperationError::engine_invariant_failed(
                request_id,
                None,
                "admitted authored clock bound exceeds the Yrs clock domain",
            )
        })?;
        let (history_update, mut next_derived_state, next_candidate_state_vector) = {
            let candidate_plan = {
                #[cfg(test)]
                check_compiled_commit_preparation_stage_for_test(
                    request_id,
                    CompiledCommitPreparationStage::DocumentValidation,
                )?;
                let txn = candidate_doc.transact();
                let candidate_plan = mutation_plan
                    .clone()
                    .rebind_to_equivalent_store(request_id, &txn)?;
                preflight_mutation_plan(request_id, &candidate_plan, &txn)?;
                candidate_plan
            };
            {
                let mut txn = candidate_doc.transact_mut();
                execute_mutation_plan(candidate_plan, &mut txn);
            }
            let txn = candidate_doc.transact();
            let fragment = txn
                .get_xml_fragment(self.fragment_name.as_str())
                .ok_or_else(|| {
                    super::OperationError::engine_invariant_failed(
                        request_id,
                        None,
                        "prepared commit candidate lost its configured Yrs fragment",
                    )
                })?;
            #[cfg(test)]
            check_compiled_commit_preparation_stage_for_test(
                request_id,
                CompiledCommitPreparationStage::HistoryUpdateEncoding,
            )?;
            let history_update = txn.encode_state_as_update_v1(&candidate_state_vector);
            if history_update.len() > encoded_growth_bound {
                return Err(super::OperationError::engine_invariant_failed(
                    request_id,
                    None,
                    "prepared commit candidate exceeded the admitted encoded growth bound",
                ));
            }
            // Yrs can elide redundant formatting structs when the requested
            // attributes are already active. The compiler's authored units are
            // therefore an admitted hard ceiling, while this private execution
            // supplies the exact next seal. Only the local client may advance.
            let next_candidate_state_vector = seal_candidate_state_vector(
                request_id,
                &candidate_state_vector,
                txn.state_vector(),
                self.doc.client_id(),
                authored_clock_bound,
            )?;
            if prepared_mutation_lookup_seed.is_none() {
                let candidate_seed = super::mutation::MutationLookupSeed::build(
                    request_id,
                    &txn,
                    &fragment,
                    &self.schema,
                    &preview,
                    &self.resource_limits,
                    &self.editing_limits,
                    self.max_length,
                    &self.schema_fingerprint,
                    next_yrs_state_epoch,
                    next_document_revision,
                )?
                .with_canonical_artifact(&canonical_artifact);
                prepared_mutation_lookup_seed =
                    Some(Arc::new(candidate_seed.rebind_authoritative_store(
                        commit_authority.txn(),
                        commit_authority.fragment(),
                        &self.schema_fingerprint,
                        next_yrs_state_epoch,
                        next_document_revision,
                    )));
            }
            #[cfg(test)]
            check_compiled_commit_preparation_stage_for_test(
                request_id,
                CompiledCommitPreparationStage::DerivedStateBuild,
            )?;
            let explicit_relative_selection = match (&selection_plan, &prepared_selection_state) {
                (SelectionPlan::Explicit(_), Some(prepared)) => Some(prepared.relative().clone()),
                (SelectionPlan::Explicit(_), None)
                    if matches!(
                        relative_selection_plan,
                        RelativeSelectionPlan::Precomputed { .. }
                    ) =>
                {
                    let RelativeSelectionPlan::Precomputed { relative, .. } =
                        &relative_selection_plan
                    else {
                        unreachable!()
                    };
                    Some(relative.clone())
                }
                (SelectionPlan::Explicit(selection), None) => Some(operation_result_to_relative(
                    &txn,
                    &fragment,
                    selection,
                    &self.schema,
                )),
                (SelectionPlan::Mapped(_), _) | (SelectionPlan::Preserve, _) => None,
            };
            #[cfg(test)]
            check_compiled_commit_preparation_stage_for_test(
                request_id,
                CompiledCommitPreparationStage::SelectionFinalization,
            )?;
            let preserved_fallback = match &relative_selection_plan {
                RelativeSelectionPlan::PreserveWithFallback(selection) => Some(selection),
                RelativeSelectionPlan::Precomputed { fallback, .. } => Some(fallback),
                _ => None,
            };
            let strict_fallback_affinity = matches!(
                relative_selection_plan,
                RelativeSelectionPlan::Precomputed { .. }
            );
            let next = self
                .derived_state
                .as_ref()
                .and_then(|state| {
                    state.after_document_change(
                        preview.clone(),
                        canonical_artifact,
                        &txn,
                        &fragment,
                        &self.schema,
                        &self.schema_fingerprint,
                        &self.resource_limits,
                        &self.editing_limits,
                        self.max_length,
                        next_render_blocks,
                        preview_derivations,
                        &composed_map,
                        position_update_mode,
                        &affected_top_level_blocks,
                        explicit_relative_selection.as_ref(),
                        preserved_fallback,
                        strict_fallback_affinity,
                        prepared_mutation_lookup_seed,
                        prepared_selection_state,
                        finalized_derived_evidence,
                        next_document_revision,
                        next_state_revision,
                        next_yrs_state_epoch,
                    )
                })
                .ok_or_else(|| {
                    super::OperationError::engine_invariant_failed(
                        request_id,
                        None,
                        "prepared candidate must produce exact next derived editor state",
                    )
                })?;
            (history_update, next, next_candidate_state_vector)
        };
        let StoredMarksPlan::Set(stored_marks) = stored_marks_plan else {
            unreachable!()
        };
        next_derived_state.stored_marks = stored_marks;
        let prepared_active_state_certificate = prepared_active_state_install.and_then(|install| {
            let authority =
                super::prepared_admission::InstalledDerivedStateAuthority::new(&next_derived_state);
            super::derived_state::DerivedStateCache::prepare_active_state_certificate(
                install,
                &authority,
                &self.resource_limits,
                &self.editing_limits,
                self.max_length,
                next_yrs_state_epoch,
            )
        });
        let active_state_installed = prepared_active_state_certificate.is_some();
        if let Some(certificate) = prepared_active_state_certificate {
            next_derived_state.install_active_state_certificate(certificate);
        }
        let publish_active_state_drop = had_active_state_certificate && !active_state_installed;
        let history_after = if captures_history {
            let history_after_template = history_after_template
                .expect("captured history has an admitted after-state template");
            let document_snapshot = if let Some(retained_bytes) =
                history_after_template.document_snapshot_retained_bytes
            {
                #[cfg(test)]
                check_compiled_commit_preparation_stage_for_test(
                    request_id,
                    CompiledCommitPreparationStage::HistorySnapshotConstruction,
                )?;
                Some(next_derived_state.capture_history_document_snapshot(
                    &self.resource_limits,
                    &self.editing_limits,
                    self.max_length,
                    &self.fragment_name,
                    self.scope.as_ref(),
                    retained_bytes,
                ))
            } else {
                None
            };
            Some(history_after_template.seal(
                next_derived_state.relative_selection.clone(),
                next_derived_state.resolved_selection.clone(),
                document_snapshot,
            ))
        } else {
            None
        };
        debug_assert_eq!(next_derived_state.document_revision, next_document_revision);
        if let Some(result) = &mut result {
            result.request_id = request_id;
            result.origin = origin;
            result.changed = true;
            result.document_revision = next_document_revision;
            result.state_revision = next_state_revision;
            result.selection = next_derived_state.resolved_selection.clone();
            result.history_state = crate::editor_state::HistoryState {
                can_undo: captures_history || self.can_undo(),
                can_redo: !captures_history && self.can_redo(),
            };
        }
        drop(commit_authority);
        drop(authority_txn);
        drop(authority_doc);
        drop(prepared_context);
        let next_candidate_cache = Some(PreparedCandidateCache {
            doc: candidate_doc,
            state_vector: next_candidate_state_vector,
            staged_lookup_seed: None,
            document_revision: next_document_revision,
            yrs_state_epoch: next_yrs_state_epoch,
            encoded_state_seal: None,
        });
        let mut prepared = PreparedCompiledCommit {
            request_id,
            origin,
            history_policy,
            history: Some(prepared_history),
            mutation_plan: Some(mutation_plan),
            history_update,
            history_after,
            next_derived_state: Some(next_derived_state),
            next_durable_client_ids,
            next_document_revision,
            next_state_revision,
            next_yrs_state_epoch,
            publish_active_state_install: active_state_installed,
            publish_active_state_drop,
            result,
            next_candidate_cache,
        };
        // Frozen local mutation flow: reserve bounded outbox count/bytes and
        // stage the candidate-captured Update-v1 from the compiler's
        // conservative bound BEFORE the irreversible Yrs write. Saturation or
        // reservation failure rejects here atomically; the invariant check on
        // `history_update` above already proved `actual <= admitted bound`.
        outbound.reserve_and_stage(
            prepared.request_id,
            outbound_update_upper_bound,
            &prepared.history_update,
        )?;
        self.execute_prepared_yrs_write(&mut prepared);
        let committed = self.install_prepared_changed_commit(prepared);
        outbound.commit_staged();
        Ok(committed)
    }

    fn execute_prepared_yrs_write(&mut self, prepared: &mut PreparedCompiledCommit) {
        let yrs_origin = match prepared
            .history
            .as_ref()
            .expect("changed commit has prepared history execution")
        {
            PreparedCompiledHistory::Recorded(admission) => admission.yrs_origin(),
            PreparedCompiledHistory::Excluded(admission) => admission.yrs_origin(),
        };
        #[cfg(test)]
        mark_compiled_commit_durable_write_for_test();
        if matches!(prepared.history, Some(PreparedCompiledHistory::Recorded(_))) {
            let Some(PreparedCompiledHistory::Recorded(admission)) = prepared.history.take() else {
                unreachable!()
            };
            self.history.begin_prepared_recorded(admission);
        }
        let mutation_plan = prepared
            .mutation_plan
            .take()
            .expect("changed commit owns one deterministic mutation plan");
        let mut txn = self.doc.transact_mut_with(yrs_origin);
        execute_mutation_plan(mutation_plan, &mut txn);
    }

    fn install_prepared_changed_commit(
        &mut self,
        mut prepared: PreparedCompiledCommit,
    ) -> (
        super::TransactionCommit,
        Option<super::TypedTransactionResult>,
    ) {
        if let Some(after) = prepared.history_after.take() {
            self.history.finish_capture(after, prepared.history_update);
        } else {
            let Some(PreparedCompiledHistory::Excluded(admission)) = prepared.history.take() else {
                unreachable!("excluded changed commit retains its prepared admission")
            };
            self.history
                .finish_prepared_excluded(admission, prepared.history_update);
            if prepared.history_policy == super::HistoryPolicy::Boundary {
                self.history.force_next_capture_boundary();
            }
        }
        let next_derived_state = prepared
            .next_derived_state
            .take()
            .expect("changed commit owns prepared derived state");
        #[cfg(test)]
        let installed_unavailable_lookup_seed = next_derived_state
            .mutation_lookup_seed
            .is_unavailable_for_test();
        self.derived_state = Some(next_derived_state);
        if prepared.publish_active_state_install {
            super::derived_state::record_active_state_cache_install();
        }
        if prepared.publish_active_state_drop {
            super::derived_state::record_active_state_cache_drop();
        }
        #[cfg(test)]
        if installed_unavailable_lookup_seed {
            super::mutation::record_unavailable_lookup_seed_install_for_test();
        }
        self.durable_client_ids = prepared.next_durable_client_ids;
        self.revision = prepared.next_document_revision;
        self.state_revision = prepared.next_state_revision;
        self.yrs_state_epoch = prepared.next_yrs_state_epoch;
        self.last_committed_origin = Some(prepared.origin);
        self.document_origin = prepared.origin.into();
        self.prepared_candidate_cache = prepared.next_candidate_cache.take();
        let commit = super::TransactionCommit {
            request_id: prepared.request_id,
            changed: true,
            document_revision: prepared.next_document_revision,
            state_revision: prepared.next_state_revision,
            origin: prepared.origin,
        };
        (commit, prepared.result)
    }

    pub fn export_snapshot(&self) -> YrsEngineResult<DocumentSnapshot> {
        crate::boundary::with_document_stack(|| self.export_snapshot_inner())
    }

    fn export_snapshot_inner(&self) -> YrsEngineResult<DocumentSnapshot> {
        let scope = self.scope.as_ref().ok_or_else(|| {
            snapshot_error(
                "SNAPSHOT_SCOPE_MISMATCH",
                "document scope is required to export a snapshot",
                "documentId",
            )
        })?;
        if !self.is_ready() {
            return Err(snapshot_error(
                "DOCUMENT_INVALID",
                "an awaiting document cannot be exported as a snapshot",
                "encodedState",
            ));
        }
        let encoded_state =
            encode_state_bounded(&self.doc, &self.resource_limits).map_err(|error| {
                YrsEngineError::limit(
                    "DOCUMENT_LIMIT_EXCEEDED",
                    error
                        .limit
                        .unwrap_or(self.resource_limits.max_encoded_state_bytes),
                    error
                        .actual
                        .unwrap_or(self.resource_limits.max_encoded_state_bytes),
                )
                .with_details(json!({ "field": "encodedState" }))
            })?;
        validate_snapshot_envelope_output(
            scope,
            &self.fragment_name,
            &self.schema_fingerprint,
            encoded_state.len(),
            &self.resource_limits,
        )?;

        Ok(DocumentSnapshot {
            format_version: SNAPSHOT_FORMAT_VERSION,
            document_id: scope.document_id.clone(),
            lineage_id: scope.lineage_id.clone(),
            fragment_name: self.fragment_name.clone(),
            schema_fingerprint: self.schema_fingerprint.clone(),
            encoded_state,
        })
    }

    pub fn restore_snapshot(
        &mut self,
        snapshot: &DocumentSnapshot,
    ) -> YrsEngineResult<EngineCommit> {
        crate::boundary::with_document_stack(|| self.restore_snapshot_inner(snapshot))
    }

    fn restore_snapshot_inner(
        &mut self,
        snapshot: &DocumentSnapshot,
    ) -> YrsEngineResult<EngineCommit> {
        self.validate_snapshot_manifest(snapshot)?;

        let current_state = encode_state_bounded(&self.doc, &self.resource_limits)?;
        if self.is_ready() && current_state == snapshot.encoded_state {
            self.quarantined_remote_update = None;
            self.reset_history_binding();
            return Ok(EngineCommit {
                changed: false,
                revision: self.revision,
            });
        }

        preflight_update_v1(&snapshot.encoded_state, &self.resource_limits)?;
        let candidate = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.build_snapshot_candidate(snapshot)
        }));
        let candidate = match candidate {
            Ok(result) => result?,
            Err(_) => {
                return Err(snapshot_error(
                    "COLLABORATION_DECODE_FAILED",
                    "Yrs rejected the encoded snapshot state",
                    "encodedState",
                ))
            }
        };
        admit_candidate_derived_output(&candidate, &self.editing_limits)?;

        let (next_revision, next_state_revision, next_yrs_state_epoch) =
            self.next_durable_revisions()?;
        let next_derived_state = build_derived_state_for_candidate(
            &candidate,
            &self.schema,
            &self.resource_limits,
            &self.editing_limits,
            self.max_length,
            &self.schema_fingerprint,
            &self.fragment_name,
            &self.canonical_schema,
            self.yrs_state_epoch,
            None,
            next_revision,
            next_state_revision,
            next_yrs_state_epoch,
        )?;
        self.doc = candidate.doc;
        // Store swap under a fresh client identity: stale remote peers drop
        // and the desired local state re-publishes with a fresh clock.
        if let Some(awareness) = self.awareness.as_mut() {
            awareness.rebind_for_store_swap(&self.doc);
        }
        let history_fragment = {
            let txn = self.doc.transact();
            txn.get_xml_fragment(self.fragment_name.as_str())
                .expect("validated snapshot candidate retains the history fragment")
        };
        self.history.rebind(&self.doc, &history_fragment);
        self.quarantined_remote_update = None;
        debug_assert_eq!(
            next_derived_state
                .as_ref()
                .map(|state| state.document_revision),
            Some(next_revision)
        );
        self.derived_state = next_derived_state;
        self.durable_client_ids = candidate.durable_client_ids;
        self.revision = next_revision;
        self.state_revision = next_state_revision;
        self.yrs_state_epoch = next_yrs_state_epoch;
        self.last_committed_origin = Some(TransactionOrigin::SnapshotRestore);
        self.document_origin = super::DocumentOrigin::Restore;
        self.prepared_candidate_cache = None;
        Ok(EngineCommit {
            changed: true,
            revision: self.revision,
        })
    }

    fn build_snapshot_candidate(
        &self,
        snapshot: &DocumentSnapshot,
    ) -> YrsEngineResult<CandidateDocument> {
        let (candidate_doc, durable_client_ids) = {
            let update = Update::decode_v1(&snapshot.encoded_state).map_err(|error| {
                snapshot_parse_error("COLLABORATION_DECODE_FAILED", error, "encodedState")
            })?;
            let durable_state = update.state_vector();
            let durable_client_ids = durable_state
                .iter()
                .map(|(client, _)| client.get())
                .collect();
            let candidate_doc = fresh_utf16_doc_excluding(&durable_client_ids, self.client_id());
            candidate_doc
                .transact_mut_with(TransactionOrigin::SnapshotRestore.as_yrs_origin())
                .apply_update(update)
                .map_err(|error| {
                    snapshot_parse_error("COLLABORATION_DECODE_FAILED", error, "encodedState")
                })?;
            if candidate_doc.transact().has_missing_updates() {
                return Err(snapshot_error(
                    "COLLABORATION_DECODE_FAILED",
                    "encoded snapshot contains unresolved Update-v1 dependencies",
                    "encodedState",
                ));
            }
            (candidate_doc, durable_client_ids)
        };

        let codec = YrsDocumentCodec::new(&self.schema, &self.resource_limits);
        let derived_json = {
            let txn = candidate_doc.transact();
            let fragment = txn
                .get_xml_fragment(self.fragment_name.as_str())
                .ok_or_else(|| {
                    snapshot_error(
                        "CODEC_INVARIANT_FAILED",
                        "snapshot Yrs fragment is missing",
                        "fragmentName",
                    )
                })?;
            codec
                .read_json(&fragment, &txn)
                .map_err(|error| snapshot_derived_error(error, "encodedState"))?
        };
        let derived_document = from_prosemirror_json_with_limits(
            &derived_json,
            &self.schema,
            UnknownTypeMode::Preserve,
            &self.resource_limits,
        )
        .map_err(map_json_import_error)
        .map_err(|error| {
            snapshot_derived_error(
                YrsEngineError::new("CODEC_INVARIANT_FAILED", error.to_string()),
                "encodedState",
            )
        })?;
        let derived_document = rehydrate_reserved_html_opaque(&derived_document);
        validate_import_document(&derived_document, &self.schema, &self.resource_limits)
            .map_err(|error| snapshot_derived_error(error, "encodedState"))?;
        encode_candidate_state_bounded(&candidate_doc, &self.resource_limits)
            .map_err(|error| snapshot_derived_error(error, "encodedState"))?;
        let canonical_artifact =
            self.canonical_schema
                .derive(&derived_document)
                .map_err(|error| {
                    snapshot_derived_error(
                        YrsEngineError::new("CODEC_INVARIANT_FAILED", error.to_string()),
                        "encodedState",
                    )
                })?;
        Ok(CandidateDocument {
            doc: candidate_doc,
            state: EngineDocumentState::Ready {
                document: derived_document,
                canonical_artifact,
            },
            durable_client_ids,
            validated_import: None,
            import_acceleration_eligible: false,
            import_encoded_state_receipt: None,
        })
    }

    fn validate_snapshot_manifest(&self, snapshot: &DocumentSnapshot) -> YrsEngineResult<()> {
        if snapshot.format_version != SNAPSHOT_FORMAT_VERSION {
            return Err(snapshot_error(
                "SNAPSHOT_VERSION_UNSUPPORTED",
                format!(
                    "unsupported snapshot format version {}",
                    snapshot.format_version
                ),
                "formatVersion",
            ));
        }
        let scope = self.scope.as_ref().ok_or_else(|| {
            snapshot_error(
                "SNAPSHOT_SCOPE_MISMATCH",
                "document scope is required to restore a snapshot",
                "documentId",
            )
        })?;
        if snapshot.document_id != scope.document_id {
            return Err(snapshot_error(
                "SNAPSHOT_SCOPE_MISMATCH",
                "snapshot document ID does not match the engine scope",
                "documentId",
            ));
        }
        if snapshot.lineage_id != scope.lineage_id {
            return Err(snapshot_error(
                "SNAPSHOT_LINEAGE_MISMATCH",
                "snapshot lineage ID does not match the engine scope",
                "lineageId",
            ));
        }
        if snapshot.fragment_name != self.fragment_name {
            return Err(snapshot_error(
                "SNAPSHOT_FRAGMENT_MISMATCH",
                "snapshot fragment name does not match the engine fragment",
                "fragmentName",
            ));
        }
        if snapshot.schema_fingerprint != self.schema_fingerprint {
            return Err(snapshot_error(
                "SNAPSHOT_SCHEMA_MISMATCH",
                "snapshot schema fingerprint does not match the engine schema",
                "schemaFingerprint",
            ));
        }
        let metadata_bytes = snapshot.metadata_byte_len();
        if metadata_bytes > self.resource_limits.max_input_bytes {
            return Err(YrsEngineError::limit(
                "DOCUMENT_LIMIT_EXCEEDED",
                self.resource_limits.max_input_bytes,
                metadata_bytes,
            )
            .with_details(json!({ "field": "metadata" })));
        }
        if snapshot.encoded_state.len() > self.resource_limits.max_encoded_state_bytes {
            return Err(YrsEngineError::limit(
                "DOCUMENT_LIMIT_EXCEEDED",
                self.resource_limits.max_encoded_state_bytes,
                snapshot.encoded_state.len(),
            )
            .with_details(json!({ "field": "encodedState" })));
        }
        Ok(())
    }

    pub fn import_json(
        &mut self,
        input: &str,
        origin: TransactionOrigin,
    ) -> YrsEngineResult<EngineCommit> {
        let input = BoundedInput::new(input, InputKind::DocumentJson, &self.resource_limits)?;
        let input_len = input.as_str().len();
        let value = self.parse_document_json(input.as_str())?;
        with_document_stack_for_json_container_depth(value.container_depth(), || {
            self.import_json_inner(value.as_value(), input_len, origin)
        })
    }

    fn import_json_inner(
        &mut self,
        value: &serde_json::Value,
        input_len: usize,
        origin: TransactionOrigin,
    ) -> YrsEngineResult<EngineCommit> {
        if let Some(state) = &self.derived_state {
            if crate::boundary::json_values_equal_stack_safe(
                state.canonical_artifact.value(),
                value,
            ) {
                self.quarantined_remote_update = None;
                self.reset_history_binding();
                return Ok(EngineCommit {
                    changed: false,
                    revision: self.revision,
                });
            }
        }
        let source = self.admit_validated_json_document(value, input_len)?;
        let candidate = self.build_candidate_from_document(source, origin)?;
        self.commit_candidate(candidate, origin)
    }

    fn parse_document_json(
        &self,
        input: &str,
    ) -> YrsEngineResult<crate::boundary::StackSafeJsonValue> {
        let container_limit =
            document_json_container_depth_limit(self.resource_limits.max_document_depth)
                .map_err(YrsEngineError::from)?;
        parse_json_value_stack_safe(
            input,
            container_limit,
            self.resource_limits.max_document_depth,
            "DOCUMENT_LIMIT_EXCEEDED",
            "DOCUMENT_INVALID",
        )
        .map_err(YrsEngineError::from)
    }

    pub fn import_html(
        &mut self,
        input: &str,
        options: &FromHtmlOptions,
        origin: TransactionOrigin,
    ) -> YrsEngineResult<EngineCommit> {
        crate::boundary::with_document_stack(|| self.import_html_inner(input, options, origin))
    }

    fn import_html_inner(
        &mut self,
        input: &str,
        options: &FromHtmlOptions,
        origin: TransactionOrigin,
    ) -> YrsEngineResult<EngineCommit> {
        let input = BoundedInput::new(input, InputKind::Html, &self.resource_limits)?;
        let source = self.admit_validated_html_document(input.as_str(), options)?;
        let candidate = self.build_candidate_from_document(source, origin)?;
        self.commit_candidate(candidate, origin)
    }

    /// Shared JSON admission pipeline for imports and root replacements:
    /// model parse, schema/canonical validation, and derived-output ceilings
    /// in the exact import order.
    fn admit_validated_json_document(
        &self,
        value: &serde_json::Value,
        input_len: usize,
    ) -> YrsEngineResult<ValidatedImportDocument> {
        let document = from_prosemirror_json_with_limits(
            value,
            &self.schema,
            UnknownTypeMode::Preserve,
            &self.resource_limits,
        )
        .map_err(map_json_import_error)?;
        #[cfg(test)]
        super::observability::record_import_model_parse();
        let source = ValidatedImportDocument::new(
            document,
            &self.schema,
            &self.canonical_schema,
            &self.resource_limits,
            Some(input_len),
        )?;
        admit_canonical_output(&source.canonical_artifact, &self.editing_limits)?;
        Ok(source)
    }

    /// Shared HTML admission pipeline for imports and root replacements.
    fn admit_validated_html_document(
        &self,
        input: &str,
        options: &FromHtmlOptions,
    ) -> YrsEngineResult<ValidatedImportDocument> {
        let document = from_html_with_limits(input, &self.schema, options, &self.resource_limits)
            .map_err(map_html_import_error)?;
        #[cfg(test)]
        super::observability::record_import_model_parse();
        let source = ValidatedImportDocument::new(
            document,
            &self.schema,
            &self.canonical_schema,
            &self.resource_limits,
            None,
        )?;
        admit_canonical_output(&source.canonical_artifact, &self.editing_limits)?;
        Ok(source)
    }

    /// Same-store whole-document replacement from ProseMirror JSON.
    ///
    /// Admission mirrors `import_json` exactly; the admitted document then
    /// lowers to one sealed root-window `ReplaceStructure` transaction against
    /// the existing Yrs store. No candidate `Doc` swap occurs: the client
    /// identity, GUID, offset kind, and GC setting are untouched and the local
    /// client clock strictly continues.
    #[allow(dead_code)]
    pub fn prepare_root_replacement_json(
        &mut self,
        request_id: u64,
        input: &str,
        history: super::ReplacementHistory,
    ) -> Result<super::TransactionCommit, super::RootReplacementError> {
        self.prepare_root_replacement_json_with_outbox(request_id, input, history, None)
    }

    /// [`Self::prepare_root_replacement_json`] with an optionally attached
    /// collaboration outbox for bounded outbound update capture.
    pub(crate) fn prepare_root_replacement_json_with_outbox(
        &mut self,
        request_id: u64,
        input: &str,
        history: super::ReplacementHistory,
        outbox: Option<&mut crate::collaboration_runtime::CollaborationOutbox>,
    ) -> Result<super::TransactionCommit, super::RootReplacementError> {
        let source = self.admit_root_replacement_json(input)?;
        self.commit_root_replacement(request_id, source, history, outbox)
    }

    /// Same-store whole-document replacement from HTML. See
    /// [`Self::prepare_root_replacement_json`].
    #[allow(dead_code)]
    pub fn prepare_root_replacement_html(
        &mut self,
        request_id: u64,
        input: &str,
        options: &FromHtmlOptions,
        history: super::ReplacementHistory,
    ) -> Result<super::TransactionCommit, super::RootReplacementError> {
        self.prepare_root_replacement_html_with_outbox(request_id, input, options, history, None)
    }

    /// [`Self::prepare_root_replacement_html`] with an optionally attached
    /// collaboration outbox for bounded outbound update capture.
    pub(crate) fn prepare_root_replacement_html_with_outbox(
        &mut self,
        request_id: u64,
        input: &str,
        options: &FromHtmlOptions,
        history: super::ReplacementHistory,
        outbox: Option<&mut crate::collaboration_runtime::CollaborationOutbox>,
    ) -> Result<super::TransactionCommit, super::RootReplacementError> {
        use super::RootReplacementError;
        let input = BoundedInput::new(input, InputKind::Html, &self.resource_limits)
            .map_err(|error| RootReplacementError::Admission(error.into()))?;
        let source = self
            .admit_validated_html_document(input.as_str(), options)
            .map_err(RootReplacementError::Admission)?;
        self.commit_root_replacement(request_id, source, history, outbox)
    }

    /// Shared bounded-input/parse/model admission for JSON root replacement,
    /// used by both the commit path and the outbound-bound probe.
    fn admit_root_replacement_json(
        &self,
        input: &str,
    ) -> Result<ValidatedImportDocument, super::RootReplacementError> {
        use super::RootReplacementError;
        let input = BoundedInput::new(input, InputKind::DocumentJson, &self.resource_limits)
            .map_err(|error| RootReplacementError::Admission(error.into()))?;
        let value = self
            .parse_document_json(input.as_str())
            .map_err(RootReplacementError::Admission)?;
        self.admit_validated_json_document(value.as_value(), input.as_str().len())
            .map_err(RootReplacementError::Admission)
    }

    /// The sealed whole-root `ReplaceStructure` transaction for an admitted
    /// replacement document, shared by the commit path and the probe so the
    /// probed conservative bound is the bound the commit reserves.
    fn root_replacement_transaction(
        &self,
        request_id: u64,
        source: &ValidatedImportDocument,
        history: super::ReplacementHistory,
    ) -> Result<super::TypedTransaction, super::RootReplacementError> {
        use super::RootReplacementError;
        let current = self.document().ok_or_else(|| {
            RootReplacementError::Transaction(super::OperationError::engine_not_ready(request_id))
        })?;
        let root_children = u32::try_from(current.root().child_count()).map_err(|_| {
            RootReplacementError::Transaction(super::OperationError::engine_invariant_failed(
                request_id,
                None,
                "root child count exceeds the addressable replacement window",
            ))
        })?;
        let content = source
            .document
            .root()
            .content()
            .cloned()
            .unwrap_or_else(crate::model::Fragment::empty);
        let history_policy = match history {
            super::ReplacementHistory::UndoableBoundary => super::HistoryPolicy::Boundary,
            super::ReplacementHistory::ResetAndClear => super::HistoryPolicy::Skip,
        };
        Ok(super::TypedTransaction {
            request_id,
            base_document_revision: self.revision,
            origin: TransactionOrigin::LocalApi,
            operations: vec![super::TypedOperation::ReplaceStructure(
                super::StructuralReplacement::new(
                    Vec::new(),
                    0,
                    root_children,
                    content,
                    Selection::cursor(0),
                ),
            )],
            selection_intent: super::SelectionIntent::UseOperationResult,
            history_policy,
        })
    }

    /// Lower an admitted replacement document to one sealed whole-root
    /// `ReplaceStructure` transaction and apply the requested history class.
    fn commit_root_replacement(
        &mut self,
        request_id: u64,
        source: ValidatedImportDocument,
        history: super::ReplacementHistory,
        outbox: Option<&mut crate::collaboration_runtime::CollaborationOutbox>,
    ) -> Result<super::TransactionCommit, super::RootReplacementError> {
        use super::RootReplacementError;
        let transaction = self.root_replacement_transaction(request_id, &source, history)?;
        let (commit, _) = self
            .apply_typed_transaction_with_staged_context(
                transaction,
                false,
                &mut OutboundUpdateSink::from_optional_outbox(outbox),
            )
            .map_err(RootReplacementError::Transaction)?;
        if history == super::ReplacementHistory::ResetAndClear {
            self.reset_history_binding();
        }
        Ok(commit)
    }

    /// Production probe: the conservative outbound Update-v1 bound the JSON
    /// root-replacement commit would reserve, computed from the identical
    /// admission and compilation without committing anything.
    #[allow(dead_code)]
    pub(crate) fn probe_root_replacement_json_outbound_upper_bound(
        &self,
        request_id: u64,
        input: &str,
        history: super::ReplacementHistory,
    ) -> Result<usize, super::RootReplacementError> {
        let source = self.admit_root_replacement_json(input)?;
        let transaction = self.root_replacement_transaction(request_id, &source, history)?;
        self.compile_typed_transaction(transaction)
            .map(|compiled| compiled.outbound_update_upper_bound())
            .map_err(super::RootReplacementError::Transaction)
    }

    /// Production probe: the conservative outbound bound one typed transaction
    /// would reserve (compile only, no commit).
    #[allow(dead_code)]
    pub(crate) fn probe_transaction_outbound_upper_bound(
        &self,
        transaction: super::TypedTransaction,
    ) -> super::OperationResult<usize> {
        Ok(self
            .compile_typed_transaction(transaction)?
            .outbound_update_upper_bound())
    }

    /// Production probe: the conservative outbound bound one planned command
    /// would reserve; `None` when the command is not applicable or lowers to
    /// a selection-only plan (which reserves nothing).
    #[allow(dead_code)]
    pub(crate) fn probe_command_outbound_upper_bound(
        &self,
        request_id: u64,
        command: super::TypedCommand,
    ) -> super::OperationResult<Option<usize>> {
        match self.plan_command(request_id, command)? {
            super::CommandPlan::NotApplicable | super::CommandPlan::SelectionOnly(_) => Ok(None),
            super::CommandPlan::Transaction(transaction) => self
                .probe_transaction_outbound_upper_bound(transaction)
                .map(Some),
        }
    }

    /// Production probe: the exact outbound Update-v1 length the next history
    /// pop would capture and reserve (`None` when nothing can pop). The pop
    /// path's conservative bound is this exact captured length.
    #[allow(dead_code)]
    pub(crate) fn probe_history_pop_outbound_bytes(
        &self,
        request_id: u64,
        undoing: bool,
    ) -> super::OperationResult<Option<usize>> {
        let Some(prepared) = self.prepare_history_pop(request_id, undoing, false)? else {
            return Ok(None);
        };
        let live_state_vector = self.doc.transact().state_vector();
        let captured_len = {
            let candidate_txn = prepared.candidate_doc.transact();
            candidate_txn
                .encode_state_as_update_v1(&live_state_vector)
                .len()
        };
        Ok(Some(captured_len))
    }

    fn build_candidate_from_document(
        &self,
        source: ValidatedImportDocument,
        origin: TransactionOrigin,
    ) -> YrsEngineResult<CandidateDocument> {
        let doc = fresh_utf16_doc_excluding(&self.durable_client_ids, self.client_id());
        self.build_candidate_from_document_in_doc(source, origin, doc)
    }

    fn build_candidate_from_document_in_doc(
        &self,
        source: ValidatedImportDocument,
        origin: TransactionOrigin,
        doc: Doc,
    ) -> YrsEngineResult<CandidateDocument> {
        let ValidatedImportDocument {
            document: source_document,
            canonical_artifact,
            validation,
            carry_import_encoded_state_receipt,
        } = source;
        let empty_json = json!({
            "type": self.schema.doc_node_type(),
            "content": [],
        });
        let codec = YrsDocumentCodec::new(&self.schema, &self.resource_limits);
        let import_delete_set_is_empty = {
            let mut txn = doc.transact_mut_with(origin.as_yrs_origin());
            let fragment = txn.get_or_insert_xml_fragment(self.fragment_name.as_str());
            codec.apply_json(&fragment, &mut txn, &empty_json, canonical_artifact.value())?;
            txn.delete_set().is_empty()
        };

        let (matches_canonical_projection, lookup_materialization) = {
            let txn = doc.transact();
            let fragment = txn
                .get_xml_fragment(self.fragment_name.as_str())
                .ok_or_else(candidate_invariant_error)?;
            let (matches, lookup_materialization) = codec.matches_validated_json_with_lookup(
                &fragment,
                &txn,
                canonical_artifact.value(),
            );
            (matches?, lookup_materialization)
        };
        if !matches_canonical_projection {
            return Err(candidate_invariant_parse_error(
                "derived JSON does not match the admitted canonical artifact",
                "candidate codec round-trip changed the canonical projection",
            ));
        }
        let encoded_state = encode_candidate_state_bounded(&doc, &self.resource_limits)?;
        // The mandatory bounded encode above is candidate admission, not an
        // optimization. Retain it as an acceleration receipt only when the
        // exact codec traversal found a localized mutation target. If fused
        // collection failed, stay conservative and preserve the ordinary
        // receipt/fallback path; a zero-target payload is positive evidence
        // that a private replica cannot accelerate the first mutation.
        let import_acceleration_eligible = carry_import_encoded_state_receipt
            && lookup_materialization
                .as_ref()
                .is_none_or(|materialization| materialization.accelerates_localized_mutation());
        let import_encoded_state_receipt = if import_acceleration_eligible {
            ImportEncodedStateReceipt::mint(
                &doc,
                &self.fragment_name,
                encoded_state,
                import_delete_set_is_empty,
                lookup_materialization,
                &source_document,
                &canonical_artifact,
                &self.resource_limits,
                &self.editing_limits,
                self.max_length,
                &self.schema,
            )
        } else {
            None
        };
        let durable_client_ids = HashSet::from([doc.client_id().get()]);
        Ok(CandidateDocument {
            doc,
            state: EngineDocumentState::Ready {
                document: source_document,
                canonical_artifact,
            },
            durable_client_ids,
            validated_import: Some(validation),
            import_acceleration_eligible,
            import_encoded_state_receipt,
        })
    }

    fn commit_candidate(
        &mut self,
        candidate: CandidateDocument,
        origin: TransactionOrigin,
    ) -> YrsEngineResult<EngineCommit> {
        admit_candidate_derived_output(&candidate, &self.editing_limits)?;
        admit_candidate_max_length(&candidate, self.max_length)?;
        let candidate_document = match &candidate.state {
            EngineDocumentState::Ready { document, .. } => document,
            EngineDocumentState::AwaitingRemote => {
                unreachable!("imports always build ready candidates")
            }
        };
        let unchanged = self.document() == Some(candidate_document);
        if unchanged {
            self.quarantined_remote_update = None;
            self.reset_history_binding();
            return Ok(EngineCommit {
                changed: false,
                revision: self.revision,
            });
        }

        let (next_revision, next_state_revision, next_yrs_state_epoch) =
            self.next_durable_revisions()?;
        let validated_evidence = candidate
            .validated_import
            .as_ref()
            .map(|validation| {
                let EngineDocumentState::Ready {
                    document,
                    canonical_artifact,
                } = &candidate.state
                else {
                    unreachable!("validated imports are always ready")
                };
                let txn = candidate.doc.transact();
                let fragment = txn
                    .get_xml_fragment(self.fragment_name.as_str())
                    .ok_or_else(candidate_invariant_error)?;
                ValidatedDocumentEvidence::mint(
                    document,
                    &validation.source_root,
                    canonical_artifact,
                    validation.report,
                    &self.resource_limits,
                    &self.editing_limits,
                    self.max_length,
                    &self.schema_fingerprint,
                    &self.canonical_schema,
                    &self.fragment_name,
                    &txn,
                    &fragment,
                    self.yrs_state_epoch,
                    next_revision,
                    next_state_revision,
                    next_yrs_state_epoch,
                )
                .ok_or_else(candidate_invariant_error)
            })
            .transpose()?;
        let next_derived_state = build_derived_state_for_candidate(
            &candidate,
            &self.schema,
            &self.resource_limits,
            &self.editing_limits,
            self.max_length,
            &self.schema_fingerprint,
            &self.fragment_name,
            &self.canonical_schema,
            self.yrs_state_epoch,
            validated_evidence.as_ref(),
            next_revision,
            next_state_revision,
            next_yrs_state_epoch,
        )?;
        // A validated import deliberately installs an unavailable lookup seed
        // in authoritative derived state. Build the ready form while the
        // already-admitted candidate is still borrowed, then carry it only as
        // private, revision-sealed acceleration alongside the exact candidate
        // replica. Failure is opportunistic: the import remains successful and
        // the first mutation uses the ordinary staged hydration path.
        let mut import_encoded_state_receipt = candidate.import_encoded_state_receipt;
        let staged_lookup_seed = if candidate.import_acceleration_eligible {
            next_derived_state.as_ref().and_then(|state| {
                let txn = candidate.doc.transact();
                let fragment = txn.get_xml_fragment(self.fragment_name.as_str())?;
                let fused = import_encoded_state_receipt.as_mut().and_then(|receipt| {
                    receipt.take_matching_lookup_materialization(
                        &candidate.doc,
                        &self.fragment_name,
                        &state.document,
                        &state.canonical_artifact,
                        &self.resource_limits,
                        &self.editing_limits,
                        self.max_length,
                        &self.schema,
                        &self.schema_fingerprint,
                        next_revision,
                        next_yrs_state_epoch,
                    )
                });
                let fused_seed = fused.and_then(|receipt| {
                    super::mutation::MutationLookupSeed::from_import_materialization(
                        0,
                        receipt.materialization,
                        &txn,
                        &fragment,
                        receipt.source_document,
                        receipt.canonical_artifact,
                        receipt.resource_limits,
                        receipt.editing_limits,
                        receipt.max_length,
                        &self.schema_fingerprint,
                        receipt.yrs_state_epoch,
                        receipt.document_revision,
                    )
                    .ok()
                    .and_then(|seed| seed.try_publish_hydrated(0).ok())
                });
                fused_seed.or_else(|| {
                    super::mutation::MutationLookupSeed::build(
                        0,
                        &txn,
                        &fragment,
                        &self.schema,
                        &state.document,
                        &self.resource_limits,
                        &self.editing_limits,
                        self.max_length,
                        &self.schema_fingerprint,
                        next_yrs_state_epoch,
                        next_revision,
                    )
                    .ok()
                    .map(|seed| seed.with_canonical_artifact(&state.canonical_artifact))
                    .and_then(|seed| seed.try_publish_hydrated(0).ok())
                })
            })
        } else {
            None
        };
        // max_encoded_state_bytes remains the configurable hard ceiling for
        // eligible replica and retained-receipt work. Ineligible imports
        // install the same authoritative candidate and derived state, leaving
        // ordinary hydration/bootstrap to the first actual mutation.
        let prepared_candidate_cache = if candidate.import_acceleration_eligible {
            prepare_import_candidate_cache(
                &candidate.doc,
                &self.fragment_name,
                &self.resource_limits,
                import_encoded_state_receipt,
                staged_lookup_seed,
                next_revision,
                next_yrs_state_epoch,
            )
        } else {
            None
        };
        self.doc = candidate.doc;
        // Import swaps the store under a fresh client identity (the
        // ResetAndClear-style swap): rebind exactly like a snapshot restore.
        if let Some(awareness) = self.awareness.as_mut() {
            awareness.rebind_for_store_swap(&self.doc);
        }
        let history_fragment = {
            let txn = self.doc.transact();
            txn.get_xml_fragment(self.fragment_name.as_str())
                .expect("validated import candidate retains the history fragment")
        };
        self.history.rebind(&self.doc, &history_fragment);
        self.quarantined_remote_update = None;
        debug_assert_eq!(
            next_derived_state
                .as_ref()
                .map(|state| state.document_revision),
            Some(next_revision)
        );
        self.derived_state = next_derived_state;
        self.durable_client_ids = candidate.durable_client_ids;
        self.revision = next_revision;
        self.state_revision = next_state_revision;
        self.yrs_state_epoch = next_yrs_state_epoch;
        self.last_committed_origin = Some(origin);
        self.document_origin = origin.into();
        self.prepared_candidate_cache = prepared_candidate_cache;
        Ok(EngineCommit {
            changed: true,
            revision: self.revision,
        })
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

    fn prepare_mutation_lookup_seed(
        &self,
        request_id: u64,
    ) -> super::OperationResult<super::prepared_admission::PreparedMutationContext> {
        let state = self
            .derived_state
            .as_ref()
            .ok_or_else(|| super::OperationError::engine_not_ready(request_id))?;
        if state.document_revision != self.revision
            || state.state_revision != self.state_revision
            || state.schema_fingerprint != self.schema_fingerprint
            || state.canonical_artifact.schema_fingerprint() != self.schema_fingerprint
        {
            return Err(super::OperationError::engine_invariant_failed(
                request_id,
                None,
                "installed derived state does not match the live engine context",
            ));
        }
        let txn = self.doc.transact();
        let fragment = txn
            .get_xml_fragment(self.fragment_name.as_str())
            .ok_or_else(|| {
                super::OperationError::engine_invariant_failed(
                    request_id,
                    None,
                    "ready engine lost its mutation lookup fragment",
                )
            })?;
        let installed_seed_matches = state
            .mutation_lookup_seed
            .matches_canonical_artifact(&state.canonical_artifact)
            && state.mutation_lookup_seed.matches(
                &txn,
                &fragment,
                &state.document,
                &self.resource_limits,
                &self.editing_limits,
                self.max_length,
                &self.schema_fingerprint,
                self.yrs_state_epoch,
                self.revision,
            );
        let staged_lookup_seed = self
            .prepared_candidate_cache
            .as_ref()
            .filter(|cache| {
                cache.document_revision == self.revision
                    && cache.yrs_state_epoch == self.yrs_state_epoch
            })
            .and_then(|cache| cache.staged_lookup_seed.as_ref())
            .filter(|seed| {
                seed.matches_canonical_artifact(&state.canonical_artifact)
                    && seed.matches(
                        &txn,
                        &fragment,
                        &state.document,
                        &self.resource_limits,
                        &self.editing_limits,
                        self.max_length,
                        &self.schema_fingerprint,
                        self.yrs_state_epoch,
                        self.revision,
                    )
            })
            .cloned();
        let lookup_seed = if installed_seed_matches {
            Arc::clone(&state.mutation_lookup_seed)
        } else if let Some(seed) = staged_lookup_seed {
            #[cfg(test)]
            super::observability::record_staged_seed_preparation();
            seed
        } else {
            let target_capacity_hint = state
                .localized_text_index
                .as_ref()
                .map_or(0, |index| index.leaves().len());
            let hydrated = state
                .mutation_lookup_seed
                .hydrate_with_target_capacity_hint(
                    request_id,
                    &txn,
                    &fragment,
                    &self.schema,
                    &state.document,
                    &self.resource_limits,
                    &self.editing_limits,
                    self.max_length,
                    &self.schema_fingerprint,
                    self.yrs_state_epoch,
                    self.revision,
                    target_capacity_hint,
                )?
                .with_canonical_artifact(&state.canonical_artifact)
                .try_publish_hydrated(request_id)?;
            #[cfg(test)]
            super::observability::record_staged_seed_preparation();
            hydrated
        };
        // `authority` below performs the one exact live-store validation of
        // whichever seed source won. Avoid repeating the same binding walk
        // here; no prepared context escapes unless that authority check passes.
        let context = super::prepared_admission::PreparedMutationContext::new(
            request_id,
            state.document.clone(),
            state.canonical_artifact.clone(),
            self.revision,
            self.state_revision,
            self.yrs_state_epoch,
            self.schema_fingerprint.clone().into_boxed_str(),
            self.fragment_name.clone().into_boxed_str(),
            self.resource_limits.clone(),
            self.editing_limits.clone(),
            self.max_length,
            lookup_seed,
        );
        {
            context.authority(super::prepared_admission::LiveMutationAuthorityContext {
                request_id,
                installed: state,
                txn: &txn,
                fragment: &fragment,
                fragment_name: &self.fragment_name,
                schema_fingerprint: &self.schema_fingerprint,
                resource_limits: &self.resource_limits,
                editing_limits: &self.editing_limits,
                max_length: self.max_length,
                document_revision: self.revision,
                state_revision: self.state_revision,
                yrs_state_epoch: self.yrs_state_epoch,
            })?;
        }
        Ok(context)
    }

    fn prepare_mutation_identity(
        &self,
        context: &mut super::prepared_admission::PreparedMutationContext,
    ) -> super::OperationResult<()> {
        if context.materialized_identity().is_some() {
            return Ok(());
        }
        let state = self
            .derived_state
            .as_ref()
            .ok_or_else(|| super::OperationError::engine_not_ready(context.request_id()))?;
        let txn = self.doc.transact();
        let fragment = txn
            .get_xml_fragment(self.fragment_name.as_str())
            .ok_or_else(|| {
                super::OperationError::engine_invariant_failed(
                    context.request_id(),
                    None,
                    "ready engine lost its mutation lookup fragment",
                )
            })?;
        {
            context.authority(super::prepared_admission::LiveMutationAuthorityContext {
                request_id: context.request_id(),
                installed: state,
                txn: &txn,
                fragment: &fragment,
                fragment_name: &self.fragment_name,
                schema_fingerprint: &self.schema_fingerprint,
                resource_limits: &self.resource_limits,
                editing_limits: &self.editing_limits,
                max_length: self.max_length,
                document_revision: self.revision,
                state_revision: self.state_revision,
                yrs_state_epoch: self.yrs_state_epoch,
            })?;
        }
        let canonical_fingerprint = context.canonical_artifact().sha256();
        let canonical_serialized_len = context.canonical_artifact().serialized_len();
        if !state.matches_materialized_mutation_identity(
            context.canonical_artifact(),
            canonical_fingerprint,
            canonical_serialized_len,
            &self.resource_limits,
            &self.schema_fingerprint,
            self.revision,
            self.state_revision,
            self.yrs_state_epoch,
        ) {
            // Identity is optional cached evidence. Runtime limit changes can
            // legitimately make the installed validation certificate
            // ineligible for reuse; leave identity absent so compilation uses
            // its full validation path and preserves established error order.
            return Ok(());
        }
        context.set_materialized_identity(
            super::prepared_admission::MaterializedMutationIdentity {
                canonical_fingerprint,
                canonical_serialized_len,
            },
        );
        #[cfg(test)]
        super::observability::record_staged_identity_materialization();
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare_mutation_lookup_transition_with_authority<T: ReadTxn>(
        &self,
        request_id: u64,
        authority: &dyn super::prepared_admission::DerivedStateAuthority,
        transition: &MutationLookupTransition,
        txn: &T,
        fragment: &XmlFragmentRef,
        preview: &Document,
        canonical_artifact: &super::canonical::CanonicalArtifact,
        next_yrs_state_epoch: u64,
        next_document_revision: u64,
    ) -> super::OperationResult<Arc<super::mutation::MutationLookupSeed>> {
        let current = authority.installed();
        let seed = authority.lookup_seed(request_id)?;
        let prepared = match transition {
            MutationLookupTransition::Promote(promotion) => seed.prepare_promotion(
                txn,
                fragment,
                promotion,
                &current.document,
                preview,
                &self.resource_limits,
                &self.editing_limits,
                self.max_length,
                &self.schema_fingerprint,
                self.yrs_state_epoch,
                self.revision,
                next_yrs_state_epoch,
                next_document_revision,
            )?,
            MutationLookupTransition::Invalidate {
                request_id: transition_request_id,
            } => {
                if *transition_request_id != request_id {
                    return Err(super::OperationError::engine_invariant_failed(
                        request_id,
                        None,
                        "localized mutation lookup invalidation request is stale",
                    ));
                }
                seed.prepare_unavailable_transition(
                    request_id,
                    txn,
                    fragment,
                    &current.document,
                    preview,
                    &self.resource_limits,
                    &self.editing_limits,
                    self.max_length,
                    &self.schema_fingerprint,
                    self.yrs_state_epoch,
                    self.revision,
                    next_yrs_state_epoch,
                    next_document_revision,
                )?
            }
        };
        Ok(Arc::new(
            prepared.with_canonical_artifact(canonical_artifact),
        ))
    }

    #[cfg(test)]
    fn finalize_deferred_for_test(
        &self,
        deferred: super::prepared_admission::DeferredCommandAdmission,
        context: &super::prepared_admission::PreparedMutationContext,
        transaction: &super::TypedTransaction,
        expected_preview: &crate::model::Document,
    ) -> super::OperationResult<super::compiler::PreparedSemanticAdmission> {
        let state = self
            .derived_state
            .as_ref()
            .ok_or_else(|| super::OperationError::engine_not_ready(transaction.request_id))?;
        let txn = self.doc.transact();
        let fragment = txn
            .get_xml_fragment(self.fragment_name.as_str())
            .ok_or_else(|| {
                super::OperationError::engine_invariant_failed(
                    transaction.request_id,
                    None,
                    "ready engine lost its deferred-finalization fragment",
                )
            })?;
        let staged =
            context.authority(super::prepared_admission::LiveMutationAuthorityContext {
                request_id: transaction.request_id,
                installed: state,
                txn: &txn,
                fragment: &fragment,
                fragment_name: &self.fragment_name,
                schema_fingerprint: &self.schema_fingerprint,
                resource_limits: &self.resource_limits,
                editing_limits: &self.editing_limits,
                max_length: self.max_length,
                document_revision: self.revision,
                state_revision: self.state_revision,
                yrs_state_epoch: self.yrs_state_epoch,
            })?;
        super::compiler::finalize_deferred_admission(
            &staged,
            deferred,
            super::compiler::PreparedSemanticLiveContext {
                transaction,
                expected_preview,
                canonical_schema: &self.canonical_schema,
            },
        )
    }

    #[cfg(test)]
    fn ensure_mutation_lookup_seed(&mut self, request_id: u64) -> super::OperationResult<()> {
        let context = self.prepare_mutation_lookup_seed(request_id)?;
        let prepared = Arc::clone(context.lookup_seed());
        let state = self.derived_state.as_mut().ok_or_else(|| {
            super::OperationError::engine_invariant_failed(
                request_id,
                None,
                "ready engine lost derived state during lookup hydration",
            )
        })?;
        if Arc::ptr_eq(&state.mutation_lookup_seed, &prepared) {
            return Ok(());
        }
        state.mutation_lookup_seed = prepared;
        #[cfg(test)]
        super::observability::record_installed_base_seed_publication();
        Ok(())
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

fn affinity_aware_mapped_selection(
    selection: &crate::selection::Selection,
    relative: &super::RelativeSelection,
    map: &crate::transform::StepMap,
    preview: &Document,
    schema: &Schema,
    prepared_position_map: Option<&PositionMap>,
) -> crate::selection::Selection {
    let mapped = match (selection, relative) {
        (
            crate::selection::Selection::Text { anchor, head },
            super::RelativeSelection::Text {
                anchor: relative_anchor,
                head: relative_head,
            },
        ) => crate::selection::Selection::text(
            map_position(map, *anchor, relative_anchor.affinity),
            map_position(map, *head, relative_head.affinity),
        ),
        (crate::selection::Selection::Node { pos }, super::RelativeSelection::Node { point }) => {
            crate::selection::Selection::node(map_position(map, *pos, point.affinity))
        }
        (crate::selection::Selection::All, super::RelativeSelection::All) => {
            crate::selection::Selection::all()
        }
        _ => selection.map(map),
    };
    let owned_position_map;
    let position_map = if let Some(prepared) = prepared_position_map {
        prepared
    } else {
        super::derived_state::record_preview_position_map_derivation();
        owned_position_map = PositionMap::build(preview, schema);
        &owned_position_map
    };
    let normalized = mapped.normalized(preview, position_map);
    match normalized {
        crate::selection::Selection::Node { pos }
            if !selectable_void_at(preview.root(), pos, 0, schema) =>
        {
            crate::selection::Selection::cursor(pos).normalized(preview, position_map)
        }
        selection => selection,
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

fn cached_transition_render_update(
    update: &crate::render::incremental::CachedRenderTransitionUpdate,
) -> super::RenderUpdate {
    match update {
        crate::render::incremental::CachedRenderTransitionUpdate::None => super::RenderUpdate::None,
        crate::render::incremental::CachedRenderTransitionUpdate::Patch(patch) => {
            super::RenderUpdate::Patch(patch.clone())
        }
        crate::render::incremental::CachedRenderTransitionUpdate::Full(blocks) => {
            super::RenderUpdate::Full(blocks.clone())
        }
    }
}

fn cached_render_operation_error(
    request_id: u64,
    resource_limits: &ResourceLimits,
    error: crate::render::incremental::CachedRenderError,
) -> super::OperationError {
    match error {
        crate::render::incremental::CachedRenderError::ResourceLimitExceeded => {
            let limit = resource_limits.max_document_nodes.saturating_mul(3);
            super::OperationError::document_limit_exceeded(
                request_id,
                None,
                "maxDocumentNodes",
                u64::try_from(limit).unwrap_or(u64::MAX),
                u64::try_from(limit.saturating_add(1)).unwrap_or(u64::MAX),
            )
        }
        crate::render::incremental::CachedRenderError::AllocationFailed
        | crate::render::incremental::CachedRenderError::PositionOverflow
        | crate::render::incremental::CachedRenderError::CacheInvariantViolation => {
            super::OperationError::engine_invariant_failed(
                request_id,
                None,
                format!("cached render preparation failed: {error:?}"),
            )
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn build_derived_state_for_candidate(
    candidate: &CandidateDocument,
    schema: &Schema,
    resource_limits: &ResourceLimits,
    editing_limits: &EditingLimits,
    max_length: Option<u32>,
    schema_fingerprint: &str,
    fragment_name: &str,
    canonical_schema: &CanonicalSchemaContext,
    engine_epoch: u64,
    validated_evidence: Option<&ValidatedDocumentEvidence>,
    document_revision: u64,
    state_revision: u64,
    yrs_state_epoch: u64,
) -> YrsEngineResult<Option<DerivedStateCache>> {
    let EngineDocumentState::Ready {
        document,
        canonical_artifact,
    } = &candidate.state
    else {
        return Ok(None);
    };
    let txn = candidate.doc.transact();
    let fragment = txn.get_xml_fragment(fragment_name).ok_or_else(|| {
        YrsEngineError::new(
            "CODEC_INVARIANT_FAILED",
            "ready Yrs document fragment is missing while deriving editor state",
        )
    })?;
    if let Some(limit) = max_length {
        let actual = canonical_artifact.text_scalar_len();
        if actual > u64::from(limit) {
            return Err(YrsEngineError::limit(
                "DOCUMENT_LIMIT_EXCEEDED",
                limit as usize,
                usize::try_from(actual).unwrap_or(usize::MAX),
            )
            .with_details(json!({ "field": "maxLength" })));
        }
    }
    let initialized = if let Some(evidence) = validated_evidence {
        DerivedStateCache::initialize_validated_candidate(
            document.clone(),
            canonical_artifact.clone(),
            &txn,
            &fragment,
            schema,
            resource_limits,
            editing_limits,
            max_length,
            schema_fingerprint,
            ValidatedCandidateContext {
                evidence,
                canonical_schema,
                fragment_name,
                engine_epoch,
            },
            document_revision,
            state_revision,
            yrs_state_epoch,
        )
    } else {
        DerivedStateCache::initialize(
            document.clone(),
            canonical_artifact.clone(),
            &txn,
            &fragment,
            schema,
            resource_limits,
            editing_limits,
            max_length,
            schema_fingerprint,
            document_revision,
            state_revision,
            yrs_state_epoch,
        )
    };
    initialized.map(Some).ok_or_else(|| {
        YrsEngineError::new(
            "CODEC_INVARIANT_FAILED",
            "ready Yrs document cannot initialize derived editor state",
        )
    })
}

fn admit_candidate_derived_output(
    candidate: &CandidateDocument,
    editing_limits: &EditingLimits,
) -> YrsEngineResult<()> {
    let EngineDocumentState::Ready {
        canonical_artifact, ..
    } = &candidate.state
    else {
        return Ok(());
    };
    admit_canonical_output(canonical_artifact, editing_limits)
}

fn admit_candidate_max_length(
    candidate: &CandidateDocument,
    max_length: Option<u32>,
) -> YrsEngineResult<()> {
    let (
        EngineDocumentState::Ready {
            canonical_artifact, ..
        },
        Some(limit),
    ) = (&candidate.state, max_length)
    else {
        return Ok(());
    };
    let actual = canonical_artifact.text_scalar_len();
    if actual > u64::from(limit) {
        return Err(YrsEngineError::limit(
            "DOCUMENT_LIMIT_EXCEEDED",
            limit as usize,
            usize::try_from(actual).unwrap_or(usize::MAX),
        )
        .with_details(json!({ "field": "maxLength" })));
    }
    Ok(())
}

fn admit_canonical_output(
    artifact: &CanonicalArtifact,
    editing_limits: &EditingLimits,
) -> YrsEngineResult<()> {
    let limit = editing_limits.max_derived_output_bytes;
    if artifact.admitted_serialized_upper_bound() <= limit {
        return Ok(());
    }
    let actual = artifact.serialized_len();
    if actual > limit {
        return Err(
            YrsEngineError::limit("DOCUMENT_LIMIT_EXCEEDED", limit, actual)
                .with_details(json!({ "field": "maxDerivedOutputBytes" })),
        );
    }
    Ok(())
}

fn snapshot_error(
    code: &'static str,
    message: impl Into<String>,
    field: &'static str,
) -> YrsEngineError {
    YrsEngineError::new(code, message).with_details(json!({ "field": field }))
}

fn history_local_state(
    state: &DerivedStateCache,
    fragment_name: &str,
    scope: Option<&DocumentScope>,
    resource_limits: &ResourceLimits,
    editing_limits: &super::EditingLimits,
    max_length: Option<u32>,
    document_snapshot_retained_bytes: Option<
        super::derived_state::HistoryDocumentSnapshotRetainedBytes,
    >,
) -> super::history::HistoryLocalState {
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
    super::history::HistoryLocalState {
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
                    .map(super::derived_state::HistoryDocumentSnapshot::retained_bytes)
                    .unwrap_or(0),
            ),
        document_snapshot,
    }
}

#[derive(Debug, Clone, Copy)]
struct HistoryDocumentSnapshotRetainedPair {
    before: super::derived_state::HistoryDocumentSnapshotRetainedBytes,
    after: super::derived_state::HistoryDocumentSnapshotRetainedBytes,
}

#[allow(clippy::too_many_arguments)]
fn history_document_snapshots_fit(
    before: &DerivedStateCache,
    after_document: &crate::model::Document,
    after_canonical_artifact: &CanonicalArtifact,
    after_derivations: &super::compiler::CompiledDocumentDerivations,
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
    after_derivations: &super::compiler::CompiledDocumentDerivations,
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
fn history_document_snapshots_fit_with_precomputed_after_charge(
    before: &DerivedStateCache,
    after_canonical_retained_bytes: usize,
    after_document_retained_bytes: usize,
    after_derivations: &super::compiler::CompiledDocumentDerivations,
    after_render_blocks: &crate::render::incremental::CachedRenderBlocks,
    after_stored_marks: Option<&[crate::model::Mark]>,
    schema_fingerprint: &str,
    fragment_name: &str,
    scope: Option<&DocumentScope>,
    metadata_limit: usize,
) -> Option<HistoryDocumentSnapshotRetainedPair> {
    let before_retained = super::derived_state::history_document_snapshot_retained_bytes(
        super::derived_state::HistoryDocumentSnapshotRetainedInput {
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
        super::derived_state::history_document_snapshot_retained_bytes_with_precomputed_document_charge(
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

fn history_snapshot_template(
    canonical_artifact: &CanonicalArtifact,
    stored_marks: Option<&[crate::model::Mark]>,
    fragment_name: &str,
    document_snapshot_retained_bytes: Option<
        super::derived_state::HistoryDocumentSnapshotRetainedBytes,
    >,
) -> super::history::HistorySnapshotTemplate {
    history_snapshot_template_from_identity(
        canonical_artifact.text_scalar_len(),
        canonical_artifact.sha256(),
        canonical_artifact.serialized_len(),
        stored_marks,
        fragment_name,
        document_snapshot_retained_bytes,
    )
}

fn history_snapshot_template_from_identity(
    text_length: u64,
    canonical_fingerprint: [u8; 32],
    derived_output_bytes: usize,
    stored_marks: Option<&[crate::model::Mark]>,
    fragment_name: &str,
    document_snapshot_retained_bytes: Option<
        super::derived_state::HistoryDocumentSnapshotRetainedBytes,
    >,
) -> super::history::HistorySnapshotTemplate {
    let retained_bytes = document_snapshot_retained_bytes
        .map(super::derived_state::HistoryDocumentSnapshotRetainedBytes::get)
        .unwrap_or(0);
    super::history::HistorySnapshotTemplate {
        stored_marks: stored_marks.map(<[crate::model::Mark]>::to_vec),
        text_length,
        canonical_fingerprint,
        derived_output_bytes,
        metadata_bytes: history_metadata_bytes(stored_marks, fragment_name)
            .saturating_add(retained_bytes),
        document_snapshot_retained_bytes,
    }
}

fn history_metadata_bytes(
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

fn history_operation_error(request_id: u64, error: YrsEngineError) -> super::OperationError {
    if let (Some(limit), Some(actual)) = (error.limit, error.actual) {
        let field = if error.code == "INPUT_LIMIT_EXCEEDED" {
            "maxEncodedStateBytes"
        } else {
            "document"
        };
        super::OperationError::document_limit_exceeded(
            request_id,
            None,
            field,
            u64::try_from(limit).unwrap_or(u64::MAX),
            u64::try_from(actual).unwrap_or(u64::MAX),
        )
    } else {
        super::OperationError::engine_invariant_failed(request_id, None, error.message)
    }
}

/// The shared `maxEncodedStateBytes` admission gate used by the remote-update
/// pipeline and the sealed state-vector/diff encoders: exact length is
/// admitted, one over rejects with the structured limit error.
fn admit_max_encoded_state_len(
    request_id: u64,
    actual_len: usize,
    max_encoded_state_bytes: usize,
) -> super::OperationResult<()> {
    if actual_len > max_encoded_state_bytes {
        return Err(super::OperationError::document_limit_exceeded(
            request_id,
            None,
            "maxEncodedStateBytes",
            max_encoded_state_bytes as u64,
            actual_len as u64,
        ));
    }
    Ok(())
}

fn remote_ingress_error(request_id: u64, error: YrsEngineError) -> super::OperationError {
    if let (Some(limit), Some(actual)) = (error.limit, error.actual) {
        let field = if error.code == "INPUT_LIMIT_EXCEEDED" {
            "maxEncodedStateBytes"
        } else {
            "encodedState"
        };
        let mut mapped = super::OperationError::document_limit_exceeded(
            request_id,
            None,
            field,
            u64::try_from(limit).unwrap_or(u64::MAX),
            u64::try_from(actual).unwrap_or(u64::MAX),
        );
        merge_operation_details(&mut mapped, error.details);
        mapped
    } else {
        super::OperationError::document_invalid(request_id, None, "update", error.message)
    }
}

fn remote_engine_error(request_id: u64, error: YrsEngineError) -> super::OperationError {
    if let (Some(limit), Some(actual)) = (error.limit, error.actual) {
        let mut mapped = super::OperationError::document_limit_exceeded(
            request_id,
            None,
            "update",
            u64::try_from(limit).unwrap_or(u64::MAX),
            u64::try_from(actual).unwrap_or(u64::MAX),
        );
        merge_operation_details(&mut mapped, error.details);
        mapped
    } else {
        super::OperationError::document_invalid(
            request_id,
            None,
            "update",
            format!("remote document cannot be decoded: {}", error.message),
        )
    }
}

fn remote_json_error(request_id: u64, error: JsonParseError) -> super::OperationError {
    match error {
        JsonParseError::ResourceLimit { limit, actual } => {
            super::OperationError::document_limit_exceeded(
                request_id,
                None,
                "update",
                u64::try_from(limit).unwrap_or(u64::MAX),
                u64::try_from(actual).unwrap_or(u64::MAX),
            )
        }
        error => {
            super::OperationError::document_invalid(request_id, None, "update", error.to_string())
        }
    }
}

fn remote_validation_error(
    request_id: u64,
    error: crate::boundary::BoundaryError,
) -> super::OperationError {
    if error.code() == "DOCUMENT_LIMIT_EXCEEDED" {
        let mut mapped = super::OperationError::document_limit_exceeded(
            request_id,
            None,
            "update",
            u64::try_from(error.limit.unwrap_or(0)).unwrap_or(u64::MAX),
            u64::try_from(error.actual.unwrap_or(0)).unwrap_or(u64::MAX),
        );
        merge_operation_details(&mut mapped, error.details);
        mapped
    } else {
        super::OperationError::document_invalid(request_id, None, "update", error.to_string())
    }
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

fn snapshot_parse_error(
    code: &'static str,
    error: impl std::fmt::Display,
    field: &'static str,
) -> YrsEngineError {
    YrsEngineError::parse(code, error).with_details(json!({ "field": field }))
}

fn snapshot_derived_error(mut error: YrsEngineError, field: &'static str) -> YrsEngineError {
    let mut details = match error.details.take() {
        Some(serde_json::Value::Object(details)) => details,
        _ => serde_json::Map::new(),
    };
    details.insert("field".into(), serde_json::Value::String(field.into()));
    error.details = Some(serde_json::Value::Object(details));
    error
}

/// Mark validation for a document arriving from outside the engine. Rank
/// order is canonicalized by the admission that follows, not required of the
/// producer; every other mark defect is still refused here.
fn validate_yrs_mark_representation<'schema>(
    document: &Document,
    schema: &'schema Schema,
) -> YrsEngineResult<CanonicalMarksEvidence<'schema>> {
    validate_importable_marks_with_evidence(document, schema).map_err(|error| YrsEngineError {
        code: error.code,
        message: error.message,
        limit: error.limit,
        actual: error.actual,
        details: error.details,
    })
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

fn validate_snapshot_envelope_output(
    scope: &DocumentScope,
    fragment_name: &str,
    schema_fingerprint: &str,
    encoded_state_bytes: usize,
    limits: &ResourceLimits,
) -> YrsEngineResult<()> {
    let metadata_bytes = scope
        .document_id
        .len()
        .saturating_add(scope.lineage_id.len())
        .saturating_add(fragment_name.len())
        .saturating_add(schema_fingerprint.len());
    let actual = metadata_bytes.saturating_add(encoded_state_bytes);
    let limit = limits
        .max_input_bytes
        .saturating_add(limits.max_encoded_state_bytes);
    if actual > limit {
        return Err(
            YrsEngineError::limit("DOCUMENT_LIMIT_EXCEEDED", limit, actual)
                .with_details(json!({ "phase": "snapshotExport" })),
        );
    }
    Ok(())
}

fn validate_import_document(
    document: &Document,
    schema: &Schema,
    resource_limits: &ResourceLimits,
) -> YrsEngineResult<()> {
    validate_import_document_report(document, schema, resource_limits).map(|_| ())
}

fn validate_import_document_report(
    document: &Document,
    schema: &Schema,
    resource_limits: &ResourceLimits,
) -> YrsEngineResult<DocumentValidationReport> {
    let root_has_doc_role = schema
        .node(document.root().node_type())
        .is_some_and(|spec| matches!(spec.role, NodeRole::Doc));
    if !root_has_doc_role {
        return Err(YrsEngineError::new(
            "DOCUMENT_INVALID",
            format!(
                "document root '{}' does not have the doc role",
                document.root().node_type()
            ),
        ));
    }
    DocumentValidator::validate_report(document, schema, resource_limits)
        .map_err(map_import_validation_error)
}

fn map_json_import_error(error: JsonParseError) -> YrsEngineError {
    match error {
        JsonParseError::ResourceLimit { limit, actual } => {
            YrsEngineError::limit("DOCUMENT_LIMIT_EXCEEDED", limit, actual)
        }
        other => YrsEngineError::parse("DOCUMENT_INVALID", other),
    }
}

fn map_html_import_error(error: ParseError) -> YrsEngineError {
    match error {
        ParseError::ResourceLimit { limit, actual } => {
            YrsEngineError::limit("DOCUMENT_LIMIT_EXCEEDED", limit, actual)
        }
        other => YrsEngineError::parse("DOCUMENT_INVALID", other),
    }
}

fn map_import_validation_error(error: crate::boundary::BoundaryError) -> YrsEngineError {
    if error.code == "DOCUMENT_LIMIT_EXCEEDED" {
        error.into()
    } else {
        YrsEngineError {
            code: "DOCUMENT_INVALID",
            message: error.message,
            limit: error.limit,
            actual: error.actual,
            details: error.details,
        }
    }
}

fn candidate_invariant_error() -> YrsEngineError {
    candidate_invariant_parse_error(
        "candidate Yrs fragment is missing",
        "candidate Yrs fragment is missing",
    )
}

fn candidate_invariant_parse_error(
    error: impl std::fmt::Display,
    message: &'static str,
) -> YrsEngineError {
    YrsEngineError::new("CODEC_INVARIANT_FAILED", format!("{message}: {error}"))
        .with_details(json!({ "phase": "candidateDerivation" }))
}

fn build_local_empty_candidate(
    schema: &Schema,
    canonical_schema: &CanonicalSchemaContext,
    fragment_name: &str,
    resource_limits: &ResourceLimits,
) -> YrsEngineResult<CandidateDocument> {
    let default_document = schema
        .default_document()
        .map_err(|error| YrsEngineError::parse("DOCUMENT_INVALID", error))?;
    DocumentValidator::validate(&default_document, schema, resource_limits)?;
    let canonical_json = to_prosemirror_json(&default_document, schema);

    let doc = utf16_doc();
    let codec = YrsDocumentCodec::new(schema, resource_limits);
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment(fragment_name);
        codec.apply_json(
            &fragment,
            &mut txn,
            &json!({
                "type": schema.doc_node_type(),
                "content": [],
            }),
            &canonical_json,
        )?;
    }

    let derived_json = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment(fragment_name).ok_or_else(|| {
            YrsEngineError::new(
                "CODEC_INVARIANT_FAILED",
                "initialized Yrs fragment is missing",
            )
        })?;
        codec.read_json(&fragment, &txn)?
    };
    let document = from_prosemirror_json_with_limits(
        &derived_json,
        schema,
        UnknownTypeMode::Error,
        resource_limits,
    )
    .map_err(|error| YrsEngineError::parse("CODEC_INVARIANT_FAILED", error))?;
    DocumentValidator::validate(&document, schema, resource_limits)?;
    let canonical_artifact = canonical_schema
        .derive(&document)
        .map_err(|error| YrsEngineError::parse("CODEC_INVARIANT_FAILED", error))?;
    encode_state_bounded(&doc, resource_limits)?;

    let durable_client_ids = HashSet::from([doc.client_id().get()]);
    Ok(CandidateDocument {
        doc,
        state: EngineDocumentState::Ready {
            document,
            canonical_artifact,
        },
        durable_client_ids,
        validated_import: None,
        import_acceleration_eligible: false,
        import_encoded_state_receipt: None,
    })
}

fn build_await_remote_candidate(
    fragment_name: &str,
    resource_limits: &ResourceLimits,
) -> YrsEngineResult<CandidateDocument> {
    let doc = utf16_doc();
    doc.get_or_insert_xml_fragment(fragment_name);
    encode_state_bounded(&doc, resource_limits)?;
    Ok(CandidateDocument {
        doc,
        state: EngineDocumentState::AwaitingRemote,
        durable_client_ids: HashSet::new(),
        validated_import: None,
        import_acceleration_eligible: false,
        import_encoded_state_receipt: None,
    })
}

fn utf16_doc() -> Doc {
    let options = Options {
        offset_kind: OffsetKind::Utf16,
        // Yrs history StackItems refer to deleted structs. Keep them available
        // in both live and candidate stores for the lifetime of an epoch.
        skip_gc: true,
        ..Options::default()
    };
    Doc::with_options(options)
}

fn fresh_utf16_doc_excluding(durable_client_ids: &HashSet<u64>, previous_client_id: u64) -> Doc {
    fresh_utf16_doc_excluding_with(durable_client_ids, previous_client_id, utf16_doc)
}

fn fresh_utf16_doc_excluding_with(
    durable_client_ids: &HashSet<u64>,
    previous_client_id: u64,
    mut candidate: impl FnMut() -> Doc,
) -> Doc {
    loop {
        let doc = candidate();
        let client_id = doc.client_id().get();
        if client_id != previous_client_id && !durable_client_ids.contains(&client_id) {
            return doc;
        }
    }
}

fn encode_state_bounded(doc: &Doc, resource_limits: &ResourceLimits) -> YrsEngineResult<Vec<u8>> {
    let txn = doc.transact();
    let encoded_state = if txn.state_vector().is_empty() {
        Vec::new()
    } else {
        txn.encode_state_as_update_v1(&StateVector::default())
    };
    if encoded_state.len() > resource_limits.max_encoded_state_bytes {
        return Err(YrsEngineError::limit(
            "INPUT_LIMIT_EXCEEDED",
            resource_limits.max_encoded_state_bytes,
            encoded_state.len(),
        ));
    }
    Ok(encoded_state)
}

fn encode_candidate_state_bounded(
    doc: &Doc,
    resource_limits: &ResourceLimits,
) -> YrsEngineResult<Vec<u8>> {
    #[cfg(test)]
    CANDIDATE_BOUNDED_STATE_ENCODINGS
        .set(CANDIDATE_BOUNDED_STATE_ENCODINGS.get().saturating_add(1));
    encode_state_bounded(doc, resource_limits).map_err(|error| {
        if error.code == "INPUT_LIMIT_EXCEEDED" {
            YrsEngineError::limit(
                "DOCUMENT_LIMIT_EXCEEDED",
                error
                    .limit
                    .unwrap_or(resource_limits.max_encoded_state_bytes),
                error
                    .actual
                    .unwrap_or(resource_limits.max_encoded_state_bytes),
            )
            .with_details(json!({ "phase": "candidateDerivation" }))
        } else {
            error
        }
    })
}

fn equivalent_private_candidate_doc(source: &Doc) -> Doc {
    Doc::with_options(Options {
        client_id: source.client_id(),
        guid: source.guid(),
        offset_kind: source.offset_kind(),
        skip_gc: source.skip_gc(),
        ..Options::default()
    })
}

fn prepare_import_candidate_cache(
    source: &Doc,
    fragment_name: &str,
    resource_limits: &ResourceLimits,
    import_encoded_state_receipt: Option<ImportEncodedStateReceipt>,
    staged_lookup_seed: Option<Arc<super::mutation::MutationLookupSeed>>,
    document_revision: u64,
    yrs_state_epoch: u64,
) -> Option<PreparedCandidateCache> {
    let admitted_receipt = import_encoded_state_receipt
        .and_then(|receipt| {
            receipt.into_matching(
                source,
                fragment_name,
                resource_limits.max_encoded_state_bytes,
            )
        })
        .and_then(
            |(encoded, state_vector, fragment_id, delete_set_is_empty)| {
                // Receipt bytes are attacker-adjacent private state. Preserve
                // the public import boundary's size-before-decode ordering if
                // that state is ever corrupted internally.
                if encoded.len() > resource_limits.max_encoded_state_bytes {
                    return None;
                }
                let update = if encoded.is_empty() {
                    None
                } else {
                    #[cfg(test)]
                    IMPORT_RECEIPT_STATE_DECODINGS
                        .set(IMPORT_RECEIPT_STATE_DECODINGS.get().saturating_add(1));
                    Some(Update::decode_v1(&encoded).ok()?)
                };
                let actual_delete_set_is_empty = update
                    .as_ref()
                    .is_none_or(|update| update.delete_set().is_empty());
                (actual_delete_set_is_empty == delete_set_is_empty).then_some((
                    encoded,
                    state_vector,
                    fragment_id,
                    update,
                    delete_set_is_empty,
                ))
            },
        );
    let (encoded, source_state_vector, source_fragment_id, update, source_delete_set_is_empty) =
        if let Some(receipt) = admitted_receipt {
            receipt
        } else {
            #[cfg(test)]
            IMPORT_CANDIDATE_STATE_ENCODINGS
                .set(IMPORT_CANDIDATE_STATE_ENCODINGS.get().saturating_add(1));
            let source_txn = source.transact();
            let source_state_vector = source_txn.state_vector();
            let encoded = if source_state_vector.is_empty() {
                Vec::new()
            } else {
                source_txn.encode_state_as_update_v1(&StateVector::default())
            };
            let source_fragment = source_txn.get_xml_fragment(fragment_name)?;
            let source_fragment_id = AsRef::<Branch>::as_ref(&source_fragment).id();
            drop(source_txn);
            let update = if encoded.is_empty() {
                None
            } else {
                Some(Update::decode_v1(&encoded).ok()?)
            };
            let source_delete_set_is_empty = update
                .as_ref()
                .is_none_or(|update| update.delete_set().is_empty());
            (
                encoded,
                source_state_vector,
                source_fragment_id,
                update,
                source_delete_set_is_empty,
            )
        };
    if encoded.len() > resource_limits.max_encoded_state_bytes {
        return None;
    }
    // Account for the authoritative store plus its one private replica under
    // the existing configurable encoded-state ceiling. Documents above half
    // that ceiling remain fully supported and simply use the exact fallback.
    if encoded.len().checked_mul(2)? > resource_limits.max_encoded_state_bytes {
        return None;
    }
    let doc = equivalent_private_candidate_doc(source);
    doc.get_or_insert_xml_fragment(fragment_name);
    if let Some(update) = update {
        doc.transact_mut().apply_update(update).ok()?;
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment(fragment_name)?;
    if txn.has_missing_updates()
        || txn.state_vector() != source_state_vector
        || AsRef::<Branch>::as_ref(&fragment).id() != source_fragment_id
    {
        return None;
    }
    drop(txn);
    let retain_encoded_state = source_delete_set_is_empty
        && retained_import_state_charge(encoded.len(), encoded.capacity())
            .is_some_and(|retained| retained <= resource_limits.max_encoded_state_bytes);
    let encoded_state_seal = retain_encoded_state.then(|| EncodedStateSeal {
        encoded_state: encoded,
        fragment_id: source_fragment_id,
        client_id: source.client_id(),
        guid: source.guid(),
        offset_kind: source.offset_kind(),
        skip_gc: source.skip_gc(),
        document_revision,
        yrs_state_epoch,
    });
    Some(PreparedCandidateCache {
        doc,
        state_vector: source_state_vector,
        staged_lookup_seed,
        document_revision,
        yrs_state_epoch,
        encoded_state_seal,
    })
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;
