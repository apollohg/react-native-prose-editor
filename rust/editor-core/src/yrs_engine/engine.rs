use serde_json::json;
use sha2::Digest;
use std::collections::HashSet;
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
                selection: &state.resolved_selection,
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

    #[allow(dead_code)] // Task 7 exposes the internal compiler through atomic application.
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
            if crate::boundary::json_values_equal_stack_safe(state.canonical_artifact.value(), value) {
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
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::sync::Arc;

    use crate::boundary::ResourceLimits;
    use crate::model::Mark;
    use crate::schema::presets::tiptap_schema;
    use crate::selection::Selection;
    use crate::serialize::FromHtmlOptions;
    use crate::serialize::{from_prosemirror_json, UnknownTypeMode};
    use crate::transform::DocumentValidator;
    use serde_json::json;
    use sha2::Digest;
    use yrs::OffsetKind;

    use yrs::branch::{Branch, BranchID, BranchPtr};
    use yrs::types::xml::{
        XmlFragment, XmlFragmentPrelim, XmlIn, XmlOut, XmlTextPrelim, XmlTextRef,
    };
    use yrs::{updates::decoder::Decode, Update};
    use yrs::{
        Assoc, ClientID, Doc, Options, ReadTxn, StateVector, StickyIndex, Transact, WriteTxn,
    };

    use crate::yrs_engine::compiler::SelectionPlan;
    use crate::yrs_engine::mutation::YrsMutationAction;
    use crate::yrs_engine::{
        Affinity, CommandPlan, EditorOffsetKind, HistoryPolicy, ResolvedSelection,
        RevisionedPosition, RevisionedRange, SelectionInput, SelectionIntent, TransactionOrigin,
        TypedCommand, TypedOperation, TypedTransaction,
    };

    use super::{
        check_compiled_commit_preparation_stage_for_test, fresh_utf16_doc_excluding_with,
        mark_compiled_commit_durable_write_for_test, reset_encoded_state_reuse_counts_for_test,
        reset_import_receipt_sha256_counts_for_test, reset_import_receipt_state_decodings_for_test,
        reset_import_state_encoding_counts_for_test,
        reset_prepared_candidate_cache_counts_for_test, seal_candidate_state_vector,
        set_compiled_commit_stage_failpoint_for_test,
        take_compiled_commit_authority_counts_for_test, take_encoded_state_reuse_counts_for_test,
        take_import_receipt_sha256_counts_for_test, take_import_receipt_state_decodings_for_test,
        take_import_state_encoding_counts_for_test, take_prepared_candidate_cache_counts_for_test,
        utf16_doc, CandidateDocument, CompiledCommitPreparationStage, CompiledTransaction,
        EngineDocumentState, OutboundUpdateSink, ValidatedImportDocument, YrsDocumentEngine,
        YrsEngineConfig,
    };

    #[test]
    fn candidate_state_vector_seal_accepts_redundant_inherited_mark_clock_below_bound() {
        let local = ClientID::new(7);
        let remote = ClientID::new(8);
        let base = StateVector::from_iter([(local, 5), (remote, 13)]);
        let actual = StateVector::from_iter([(local, 6), (remote, 13)]);

        assert_eq!(
            seal_candidate_state_vector(1, &base, actual.clone(), local, 3).unwrap(),
            actual
        );
    }

    #[test]
    fn candidate_state_vector_seal_accepts_zero_local_clock_delta() {
        let local = ClientID::new(7);
        let remote = ClientID::new(8);
        let base = StateVector::from_iter([(local, 5), (remote, 13)]);

        assert_eq!(
            seal_candidate_state_vector(1, &base, base.clone(), local, 0).unwrap(),
            base
        );
    }

    #[test]
    fn candidate_state_vector_seal_rejects_authored_clock_bound_excess() {
        let local = ClientID::new(7);
        let base = StateVector::from_iter([(local, 5)]);
        let actual = StateVector::from_iter([(local, 9)]);

        let error = seal_candidate_state_vector(1, &base, actual, local, 3)
            .expect_err("candidate local authorship above the admitted bound must reject");

        assert!(error
            .message
            .contains("exceeded its admitted authored clock bound"));
    }

    #[test]
    fn candidate_state_vector_seal_rejects_local_clock_regression() {
        let local = ClientID::new(7);
        let base = StateVector::from_iter([(local, 5)]);
        let actual = StateVector::from_iter([(local, 4)]);

        let error = seal_candidate_state_vector(1, &base, actual, local, 3)
            .expect_err("candidate local clock regression must reject");

        assert!(error.message.contains("regressed its local authored clock"));
    }

    #[test]
    fn candidate_state_vector_seal_rejects_nonlocal_clock_drift() {
        let local = ClientID::new(7);
        let remote = ClientID::new(8);
        let injected = ClientID::new(9);
        let base = StateVector::from_iter([(local, 5), (remote, 13)]);
        let actual = StateVector::from_iter([(local, 6), (remote, 14), (injected, 1)]);

        let error = seal_candidate_state_vector(1, &base, actual, local, 3)
            .expect_err("candidate nonlocal clock drift must reject");

        assert!(error.message.contains("changed a nonlocal authored clock"));
    }

    #[derive(Debug, PartialEq)]
    struct AtomicAudit {
        encoded: Vec<u8>,
        json: Option<serde_json::Value>,
        html: Option<String>,
        revision: u64,
        state_revision: u64,
        yrs_state_epoch: u64,
        client_id: u64,
        durable_client_ids: HashSet<u64>,
        origin: Option<TransactionOrigin>,
        scope: Option<crate::yrs_engine::DocumentScope>,
        fragment: String,
        fingerprint: String,
        selection: Option<crate::yrs_engine::ResolvedSelection>,
        stored_marks: Option<Vec<crate::model::Mark>>,
        can_undo: bool,
        can_redo: bool,
        retained_history_units: u64,
        replay_audit: (usize, usize, bool),
    }

    fn atomic_audit(engine: &YrsDocumentEngine) -> AtomicAudit {
        AtomicAudit {
            encoded: engine.encoded_state().unwrap(),
            json: engine.document_json(),
            html: engine.document_html(),
            revision: engine.revision,
            state_revision: engine.state_revision,
            yrs_state_epoch: engine.yrs_state_epoch,
            client_id: engine.client_id(),
            durable_client_ids: engine.durable_client_ids.clone(),
            origin: engine.last_committed_origin,
            scope: engine.scope.clone(),
            fragment: engine.fragment_name.clone(),
            fingerprint: engine.schema_fingerprint.clone(),
            selection: engine.resolved_selection().cloned(),
            stored_marks: engine.stored_marks().map(<[_]>::to_vec),
            can_undo: engine.can_undo(),
            can_redo: engine.can_redo(),
            retained_history_units: engine.history.retained_units(0).unwrap(),
            replay_audit: engine.history.replay_audit_for_test(),
        }
    }

    fn assert_prepared_candidate_state_vector_exact(engine: &YrsDocumentEngine) {
        let cache = engine
            .prepared_candidate_cache
            .as_ref()
            .expect("successful local mutation must retain its exact private candidate");
        let candidate_txn = cache.doc.transact();
        let live_txn = engine.doc.transact();
        assert_eq!(cache.state_vector, candidate_txn.state_vector());
        assert_eq!(cache.state_vector, live_txn.state_vector());
    }

    fn transaction_engine() -> YrsDocumentEngine {
        transaction_engine_with_editing_limits(crate::yrs_engine::EditingLimits::default())
    }

    fn transaction_engine_with_editing_limits(
        editing_limits: crate::yrs_engine::EditingLimits,
    ) -> YrsDocumentEngine {
        YrsDocumentEngine::new(YrsEngineConfig {
            schema: tiptap_schema(),
            fragment_name: "prosemirror".into(),
            initialization_mode: crate::yrs_engine::InitializationMode::LocalEmpty,
            resource_limits: ResourceLimits::default(),
            editing_limits,
            max_length: None,
            scope: Some(crate::yrs_engine::DocumentScope {
                document_id: "doc".into(),
                lineage_id: "lineage".into(),
            }),
        })
        .unwrap()
    }

    fn transaction_engine_with_resource_limits_and_mode(
        resource_limits: ResourceLimits,
        initialization_mode: crate::yrs_engine::InitializationMode,
    ) -> YrsDocumentEngine {
        YrsDocumentEngine::new(YrsEngineConfig {
            schema: tiptap_schema(),
            fragment_name: "prosemirror".into(),
            initialization_mode,
            resource_limits,
            editing_limits: crate::yrs_engine::EditingLimits::default(),
            max_length: None,
            scope: Some(crate::yrs_engine::DocumentScope {
                document_id: "limit-drift-doc".into(),
                lineage_id: "limit-drift-lineage".into(),
            }),
        })
        .unwrap()
    }

    fn hard_break_insert_transaction(
        engine: &YrsDocumentEngine,
        request_id: u64,
    ) -> TypedTransaction {
        TypedTransaction {
            request_id,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![TypedOperation::InsertNode {
                at: RevisionedPosition {
                    offset: 1,
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::After,
                },
                node: crate::model::Node::void("hardBreak".into(), HashMap::new()),
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Skip,
        }
    }

    fn paragraph_insert_transaction(
        engine: &YrsDocumentEngine,
        request_id: u64,
    ) -> TypedTransaction {
        TypedTransaction {
            request_id,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![TypedOperation::InsertNode {
                at: RevisionedPosition {
                    offset: engine.position_map().unwrap().total_scalars(),
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::After,
                },
                node: crate::model::Node::element(
                    "paragraph".into(),
                    HashMap::new(),
                    crate::model::Fragment::empty(),
                ),
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Skip,
        }
    }

    fn derived_evidence_matches_runtime_limits(engine: &YrsDocumentEngine) -> bool {
        let state = engine.derived_state.as_ref().unwrap();
        state.matches_materialized_mutation_identity(
            &state.canonical_artifact,
            state.canonical_artifact.sha256(),
            state.canonical_artifact.serialized_len(),
            &engine.resource_limits,
            &engine.schema_fingerprint,
            engine.revision,
            engine.state_revision,
            engine.yrs_state_epoch,
        )
    }

    fn assert_limit_drift_semantic_parity(
        drifted: &YrsDocumentEngine,
        preconfigured: &YrsDocumentEngine,
    ) {
        assert_eq!(drifted.document_json(), preconfigured.document_json());
        assert_eq!(drifted.document_html(), preconfigured.document_html());
        assert_eq!(
            drifted.resolved_selection(),
            preconfigured.resolved_selection()
        );
        assert_eq!(drifted.revision(), preconfigured.revision());
        assert_eq!(drifted.state_revision(), preconfigured.state_revision());
        assert_eq!(drifted.yrs_state_epoch, preconfigured.yrs_state_epoch);
        assert_eq!(drifted.can_undo(), preconfigured.can_undo());
        assert_eq!(drifted.can_redo(), preconfigured.can_redo());
        let drifted_state = drifted.derived_state.as_ref().unwrap();
        let preconfigured_state = preconfigured.derived_state.as_ref().unwrap();
        assert_eq!(
            drifted_state.canonical_artifact.sha256(),
            preconfigured_state.canonical_artifact.sha256()
        );
        assert!(derived_evidence_matches_runtime_limits(drifted));
        assert!(derived_evidence_matches_runtime_limits(preconfigured));
    }

    #[derive(Debug, Clone, Copy)]
    enum DeferredInsertCase {
        StrictInteriorEqualMarks,
        Empty,
        LeafBoundary,
        MarkMismatch,
        StructuralGrowth,
        UnavailableUpperBound,
        OverflowingUpperBound,
        OneOverOutputLimit,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum ExecutionAdmissionKind {
        Eager,
        Deferred,
    }

    struct DeferredInsertFixture {
        engine: YrsDocumentEngine,
        command: TypedCommand,
    }

    impl DeferredInsertFixture {
        fn execution_admission_kind(&self) -> ExecutionAdmissionKind {
            let preparation = std::cell::RefCell::new(None);
            let _ =
                self.engine
                    .plan_command_internal(65_201, self.command.clone(), Some(&preparation));
            let Some(proof) = preparation.into_inner() else {
                return ExecutionAdmissionKind::Eager;
            };
            match proof.execution_admission {
                crate::yrs_engine::prepared_admission::ExecutionSemanticAdmission::Eager(_) => {
                    ExecutionAdmissionKind::Eager
                }
                crate::yrs_engine::prepared_admission::ExecutionSemanticAdmission::Deferred(_) => {
                    ExecutionAdmissionKind::Deferred
                }
            }
        }
    }

    fn deferred_insert_fixture(case: DeferredInsertCase) -> DeferredInsertFixture {
        let mut engine = match case {
            DeferredInsertCase::StructuralGrowth => transaction_engine(),
            _ => import_document_with_unavailable_lookup_seed(),
        };
        let command = match case {
            DeferredInsertCase::Empty => TypedCommand::InsertText {
                text: String::new(),
            },
            DeferredInsertCase::OverflowingUpperBound => {
                TypedCommand::InsertText { text: "xx".into() }
            }
            _ => TypedCommand::InsertText { text: "x".into() },
        };
        if !matches!(case, DeferredInsertCase::StructuralGrowth) {
            let position = if matches!(case, DeferredInsertCase::LeafBoundary) {
                0
            } else {
                2
            };
            select_text(&mut engine, 65_202, position, position);
        }
        if matches!(case, DeferredInsertCase::MarkMismatch) {
            engine
                .apply_command(
                    65_203,
                    TypedCommand::ToggleMark {
                        mark_type: "bold".into(),
                    },
                )
                .unwrap()
                .expect("collapsed mark toggle must update stored marks");
        }
        if matches!(
            case,
            DeferredInsertCase::UnavailableUpperBound | DeferredInsertCase::OverflowingUpperBound
        ) {
            let upper_bound = if matches!(case, DeferredInsertCase::UnavailableUpperBound) {
                usize::MAX
            } else {
                usize::MAX - 1
            };
            let state = engine.derived_state.as_mut().unwrap();
            state.canonical_artifact = state
                .canonical_artifact
                .with_admission_upper_bound_for_test(upper_bound);
            engine.editing_limits.max_derived_output_bytes = usize::MAX;
        }
        if matches!(case, DeferredInsertCase::StrictInteriorEqualMarks) {
            let base = engine
                .derived_state
                .as_ref()
                .unwrap()
                .canonical_artifact
                .admitted_serialized_upper_bound();
            engine.editing_limits.max_derived_output_bytes = base + 1;
        } else if matches!(case, DeferredInsertCase::OneOverOutputLimit) {
            let base = engine
                .derived_state
                .as_ref()
                .unwrap()
                .canonical_artifact
                .admitted_serialized_upper_bound();
            engine.editing_limits.max_derived_output_bytes = base;
        }
        DeferredInsertFixture { engine, command }
    }

    fn deferred_finalization_fixture() -> (
        YrsDocumentEngine,
        crate::yrs_engine::prepared_admission::DeferredCommandAdmission,
        crate::yrs_engine::prepared_admission::PreparedMutationContext,
        TypedTransaction,
        crate::model::Document,
    ) {
        let mut engine = import_document_with_unavailable_lookup_seed();
        select_text(&mut engine, 65_240, 2, 2);
        let preparation = std::cell::RefCell::new(None);
        let CommandPlan::Transaction(transaction) = engine
            .plan_command_internal(
                65_241,
                TypedCommand::InsertText { text: "x".into() },
                Some(&preparation),
            )
            .unwrap()
        else {
            panic!("strict-interior imported insert must produce a transaction")
        };
        let proof = preparation
            .into_inner()
            .expect("strict-interior unavailable-seed insert retains preparation");
        let deferred = match proof.execution_admission {
            crate::yrs_engine::prepared_admission::ExecutionSemanticAdmission::Deferred(
                deferred,
            ) => deferred,
            crate::yrs_engine::prepared_admission::ExecutionSemanticAdmission::Eager(_) => {
                panic!("strict-interior unavailable-seed insert must defer admission")
            }
        };
        let context = engine.prepare_mutation_lookup_seed(65_241).unwrap();
        (engine, deferred, context, transaction, proof.document)
    }

    fn deferred_tamper_fixture(
        case: &str,
    ) -> (
        YrsDocumentEngine,
        crate::yrs_engine::prepared_admission::DeferredCommandAdmission,
        crate::yrs_engine::prepared_admission::PreparedMutationContext,
        TypedTransaction,
        crate::model::Document,
    ) {
        let (engine, mut deferred, context, transaction, expected_document) =
            deferred_finalization_fixture();
        deferred.tamper_for_test(case);
        (engine, deferred, context, transaction, expected_document)
    }

    struct EagerPreAdmissionErrorCase {
        name: &'static str,
        engine: YrsDocumentEngine,
        request_id: u64,
        command: TypedCommand,
        expected_error: crate::yrs_engine::OperationError,
    }

    fn eager_pre_admission_error_cases() -> Vec<EagerPreAdmissionErrorCase> {
        let mut output = import_document_with_unavailable_lookup_seed();
        select_text(&mut output, 65_220, 2, 2);
        output.editing_limits.max_derived_output_bytes = 88;

        let mut undo = import_document_with_unavailable_lookup_seed();
        select_text(&mut undo, 65_221, 2, 2);
        undo.editing_limits.max_undo_retained_units = 0;

        let retained_limits = crate::yrs_engine::EditingLimits {
            max_derived_output_bytes: 100,
            ..crate::yrs_engine::EditingLimits::default()
        };
        let mut retained_history = transaction_engine_with_editing_limits(retained_limits);
        retained_history
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        assert!(retained_history
            .derived_state
            .as_ref()
            .unwrap()
            .mutation_lookup_seed
            .is_unavailable_for_test());
        select_text(&mut retained_history, 65_222, 2, 2);
        let state = retained_history.derived_state.as_mut().unwrap();
        state.canonical_artifact = state
            .canonical_artifact
            .with_admission_upper_bound_for_test(usize::MAX);
        let retained_preparation = std::cell::RefCell::new(None);
        assert!(retained_history
            .plan_command_internal(
                65_232,
                TypedCommand::InsertText { text: "x".into() },
                Some(&retained_preparation),
            )
            .is_ok());
        let retained_proof = retained_preparation.into_inner().unwrap();
        assert_ne!(
            retained_proof
                .execution_admission
                .transaction()
                .history_policy,
            HistoryPolicy::Skip,
        );
        assert_ne!(
            retained_proof.document,
            *retained_history.document().unwrap()
        );
        let retained_history_actual =
            super::history_metadata_bytes(retained_history.stored_marks(), "prosemirror") * 2;

        let command_contract = import_document_with_unavailable_lookup_seed();

        let mut selection = import_document_with_unavailable_lookup_seed();
        let invalid = crate::yrs_engine::ResolvedPoint {
            document: 999,
            scalar: 999,
            utf16: 999,
        };
        selection.derived_state.as_mut().unwrap().resolved_selection =
            crate::yrs_engine::ResolvedSelection::Text {
                anchor: invalid,
                head: invalid,
            };

        vec![
            EagerPreAdmissionErrorCase {
                name: "exact output",
                engine: output,
                request_id: 65_230,
                command: TypedCommand::InsertText { text: "x".into() },
                expected_error: crate::yrs_engine::OperationError::document_limit_exceeded(
                    65_230,
                    Some(0),
                    "maxDerivedOutputBytes",
                    88,
                    89,
                ),
            },
            EagerPreAdmissionErrorCase {
                name: "undo",
                engine: undo,
                request_id: 65_231,
                command: TypedCommand::InsertText { text: "x".into() },
                expected_error: crate::yrs_engine::OperationError::operation_limit_exceeded(
                    65_231,
                    Some(0),
                    "maxUndoRetainedUnits",
                    0,
                    1,
                ),
            },
            EagerPreAdmissionErrorCase {
                name: "retained history",
                engine: retained_history,
                request_id: 65_232,
                command: TypedCommand::InsertText { text: "x".into() },
                expected_error: crate::yrs_engine::OperationError::document_limit_exceeded(
                    65_232,
                    None,
                    "maxDerivedOutputBytes",
                    100,
                    retained_history_actual as u64,
                ),
            },
            EagerPreAdmissionErrorCase {
                name: "command contract",
                engine: command_contract,
                request_id: 65_233,
                command: TypedCommand::ToggleMark {
                    mark_type: "missing".into(),
                },
                expected_error: crate::yrs_engine::OperationError::operation_invalid(
                    65_233,
                    0,
                    "mark",
                    "unknown mark 'missing'",
                ),
            },
            EagerPreAdmissionErrorCase {
                name: "selection",
                engine: selection,
                request_id: 65_234,
                command: TypedCommand::InsertText { text: "x".into() },
                expected_error: crate::yrs_engine::OperationError::operation_invalid(
                    65_234,
                    0,
                    "command",
                    "command simulation failed",
                ),
            },
        ]
    }

    fn insert_transaction(engine: &YrsDocumentEngine, request_id: u64) -> TypedTransaction {
        TypedTransaction {
            request_id,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![TypedOperation::InsertText {
                at: RevisionedPosition {
                    offset: 1,
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::After,
                },
                text: "x".into(),
                marks: vec![],
            }],
            selection_intent: SelectionIntent::Preserve,
            history_policy: HistoryPolicy::Skip,
        }
    }

    fn marked_insert_transaction(
        engine: &YrsDocumentEngine,
        request_id: u64,
        text: &str,
    ) -> TypedTransaction {
        TypedTransaction {
            request_id,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertText {
                at: RevisionedPosition {
                    offset: 1,
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::After,
                },
                text: text.into(),
                marks: vec![Mark::new("bold".into(), HashMap::new())],
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        }
    }

    fn import_document_with_unavailable_lookup_seed() -> YrsDocumentEngine {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        assert!(engine
            .derived_state
            .as_ref()
            .unwrap()
            .mutation_lookup_seed
            .is_unavailable_for_test());
        engine
    }

    fn hydrate_import_for_compile_test(engine: &mut YrsDocumentEngine) {
        engine.ensure_mutation_lookup_seed(0).unwrap();
        engine
            .derived_state
            .as_mut()
            .unwrap()
            .materialize_mutation_identity();
    }

    fn force_lookup_seed_unavailable(engine: &mut YrsDocumentEngine) {
        let txn = engine.doc.transact();
        let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
        let state = engine.derived_state.as_ref().unwrap();
        let unavailable =
            crate::yrs_engine::mutation::MutationLookupSeed::unavailable_for_validated_import(
                &txn,
                &fragment,
                &state.document,
                &engine.resource_limits,
                &engine.editing_limits,
                engine.max_length,
                &engine.schema_fingerprint,
                engine.yrs_state_epoch,
                engine.revision,
            )
            .with_canonical_artifact(&state.canonical_artifact);
        drop(txn);
        engine.derived_state.as_mut().unwrap().mutation_lookup_seed = Arc::new(unavailable);
    }

    #[test]
    fn apply_command_runs_one_semantic_compilation() {
        use crate::yrs_engine::canonical::{
            reset_canonical_artifact_counts_for_test, take_canonical_artifact_counts_for_test,
        };
        use crate::yrs_engine::compiler::{
            reset_semantic_compilation_count_for_test, take_semantic_compilation_count_for_test,
        };

        let mut engine = transaction_engine();
        reset_semantic_compilation_count_for_test();
        reset_canonical_artifact_counts_for_test();

        let result = engine
            .apply_command(70_001, TypedCommand::InsertText { text: "x".into() })
            .unwrap();

        assert!(result.is_some());
        assert_eq!(take_semantic_compilation_count_for_test(), 1);
        assert_eq!(take_canonical_artifact_counts_for_test(), (1, 2));
    }

    #[test]
    fn existing_text_insert_burst_hits_localized_lookup_and_promotes_without_full_rebuild() {
        use crate::yrs_engine::mutation::{
            reset_localized_lookup_counts_for_test, take_localized_lookup_counts_for_test,
        };

        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        reset_localized_lookup_counts_for_test();

        engine
            .apply_typed_transaction(insert_transaction(&engine, 70_101))
            .unwrap();
        engine
            .apply_typed_transaction(insert_transaction(&engine, 70_102))
            .unwrap();

        assert_eq!(take_localized_lookup_counts_for_test(), (0, 2, 2));
    }

    #[test]
    fn prepared_candidate_cache_reuses_one_exact_store_across_successful_insert_burst() {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let imported_cache = engine
            .prepared_candidate_cache_store_token_for_test()
            .expect("successful bounded import prepares a candidate cache");
        assert!(engine
            .derived_state
            .as_ref()
            .unwrap()
            .mutation_lookup_seed
            .is_unavailable_for_test());
        reset_prepared_candidate_cache_counts_for_test();

        engine
            .apply_typed_transaction(insert_transaction(&engine, 70_103))
            .unwrap();
        assert_prepared_candidate_state_vector_exact(&engine);
        assert_eq!(
            engine.prepared_candidate_cache_store_token_for_test(),
            Some(imported_cache),
            "the exact prepared candidate becomes the next sealed cache"
        );
        let cached_encoded = super::encode_state_bounded(
            &engine.prepared_candidate_cache.as_ref().unwrap().doc,
            &engine.resource_limits,
        )
        .unwrap();
        assert_eq!(cached_encoded, engine.encoded_state().unwrap());
        engine
            .apply_typed_transaction(insert_transaction(&engine, 70_104))
            .unwrap();
        assert_prepared_candidate_state_vector_exact(&engine);

        assert_eq!(
            engine.prepared_candidate_cache_store_token_for_test(),
            Some(imported_cache)
        );
        assert_eq!(take_prepared_candidate_cache_counts_for_test(), (2, 0));
    }

    #[test]
    fn imported_candidate_sealed_state_replaces_only_the_first_commit_full_encode() {
        let mut engine = transaction_engine();
        reset_encoded_state_reuse_counts_for_test();
        reset_import_state_encoding_counts_for_test();

        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();

        assert_eq!(take_import_state_encoding_counts_for_test(), (1, 0));
        assert_eq!(take_encoded_state_reuse_counts_for_test(), (0, 0, 0));
        engine
            .apply_typed_transaction(insert_transaction(&engine, 70_113))
            .unwrap();
        assert_eq!(
            take_encoded_state_reuse_counts_for_test(),
            (0, 0, 1),
            "the import's exact one-shot bytes must replace the first commit-time full encode"
        );

        engine
            .apply_typed_transaction(insert_transaction(&engine, 70_114))
            .unwrap();
        assert_eq!(
            take_encoded_state_reuse_counts_for_test(),
            (0, 1, 0),
            "successful mutation caches must not retain the stale import bytes"
        );
    }

    #[test]
    fn validated_html_import_carries_its_first_bounded_encode_into_the_cache() {
        let mut engine = transaction_engine();
        reset_import_state_encoding_counts_for_test();

        engine
            .import_html(
                "<p>abc</p>",
                &FromHtmlOptions::default(),
                TransactionOrigin::DocumentImport,
            )
            .unwrap();

        assert_eq!(take_import_state_encoding_counts_for_test(), (1, 0));
        assert_prepared_candidate_state_vector_exact(&engine);
    }

    #[test]
    fn import_cache_eligibility_requires_a_localized_mutation_target() {
        let empty_textblock_engine = transaction_engine();
        let empty_textblock_value = json!({
            "type": "doc",
            "content": [{ "type": "paragraph" }]
        });
        let empty_textblock_document = from_prosemirror_json(
            &empty_textblock_value,
            &empty_textblock_engine.schema,
            UnknownTypeMode::Preserve,
        )
        .unwrap();
        let empty_textblock_source = ValidatedImportDocument::new(
            empty_textblock_document,
            &empty_textblock_engine.schema,
            &empty_textblock_engine.canonical_schema,
            &empty_textblock_engine.resource_limits,
            Some(empty_textblock_value.to_string().len()),
        )
        .unwrap();
        let empty_textblock_candidate = empty_textblock_engine
            .build_candidate_from_document(
                empty_textblock_source,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        assert!(
            empty_textblock_candidate.import_acceleration_eligible,
            "the collector's trailing empty-textblock gap is a localized target"
        );
        assert!(empty_textblock_candidate
            .import_encoded_state_receipt
            .is_some());

        let mut one_text_target = transaction_engine();
        reset_import_receipt_sha256_counts_for_test();
        one_text_target
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"a"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        assert!(
            one_text_target.prepared_candidate_cache.is_some(),
            "one localized text target remains eligible"
        );
        assert_eq!(take_import_receipt_sha256_counts_for_test(), (1, 1));

        let mut known_void = transaction_engine();
        reset_import_state_encoding_counts_for_test();
        reset_import_receipt_state_decodings_for_test();
        reset_import_receipt_sha256_counts_for_test();
        known_void
            .import_json(
                r#"{"type":"doc","content":[{"type":"image","attrs":{"src":"asset://one"}}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        assert_eq!(
            take_import_state_encoding_counts_for_test(),
            (1, 0),
            "candidate admission still performs its one mandatory bounded encode"
        );
        assert_eq!(take_import_receipt_state_decodings_for_test(), 0);
        assert_eq!(take_import_receipt_sha256_counts_for_test(), (0, 0));
        assert!(
            known_void.prepared_candidate_cache.is_none(),
            "a textless void-only document has no localized target to accelerate"
        );
        assert_eq!(
            known_void.document_json().unwrap(),
            json!({
                "type": "doc",
                "content": [{
                    "type": "image",
                    "attrs": { "src": "asset://one" }
                }]
            })
        );

        for (name, value) in [
            (
                "mixedTextOpaque",
                json!({
                    "type": "doc",
                    "content": [
                        {
                            "type": "paragraph",
                            "content": [{ "type": "text", "text": "addressable" }]
                        },
                        {
                            "type": "customOpaqueBlock",
                            "attrs": { "payload": "retained" }
                        }
                    ]
                }),
            ),
            (
                "article",
                json!({
                    "type": "doc",
                    "content": [
                        {
                            "type": "paragraph",
                            "content": [{ "type": "text", "text": "Title" }]
                        },
                        {
                            "type": "paragraph",
                            "content": [{ "type": "text", "text": "Body" }]
                        }
                    ]
                }),
            ),
        ] {
            let mut engine = transaction_engine();
            engine
                .import_json(&value.to_string(), TransactionOrigin::DocumentImport)
                .unwrap();
            assert!(
                engine.prepared_candidate_cache.is_some(),
                "{name} must retain import acceleration"
            );
            assert_eq!(engine.document_json().unwrap(), value, "{name}");
        }
    }

    #[test]
    fn deferred_import_still_obeys_exact_candidate_encoded_state_ceiling() {
        fn validated_opaque_source(
            engine: &YrsDocumentEngine,
            value: &serde_json::Value,
        ) -> ValidatedImportDocument {
            let document =
                from_prosemirror_json(value, &engine.schema, UnknownTypeMode::Preserve).unwrap();
            ValidatedImportDocument::new(
                document,
                &engine.schema,
                &engine.canonical_schema,
                &engine.resource_limits,
                Some(value.to_string().len()),
            )
            .unwrap()
        }

        let value = json!({
            "type": "doc",
            "content": [{
                "type": "benchmarkOpaqueBlock",
                "attrs": { "payload": "opaque" }
            }]
        });
        let probe = transaction_engine();
        let candidate = probe
            .build_candidate_from_document(
                validated_opaque_source(&probe, &value),
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        assert!(!candidate.import_acceleration_eligible);
        assert!(candidate.import_encoded_state_receipt.is_none());
        let encoded_len = super::encode_state_bounded(&candidate.doc, &probe.resource_limits)
            .unwrap()
            .len();
        let exact_doc = super::equivalent_private_candidate_doc(&candidate.doc);
        let one_under_doc = super::equivalent_private_candidate_doc(&candidate.doc);

        let mut exact = transaction_engine();
        exact.resource_limits = ResourceLimits {
            max_encoded_state_bytes: encoded_len,
            ..exact.resource_limits.clone()
        };
        reset_import_state_encoding_counts_for_test();
        let exact_candidate = exact
            .build_candidate_from_document_in_doc(
                validated_opaque_source(&exact, &value),
                TransactionOrigin::DocumentImport,
                exact_doc,
            )
            .expect("the exact authoritative candidate byte ceiling must admit");
        assert!(!exact_candidate.import_acceleration_eligible);
        assert_eq!(take_import_state_encoding_counts_for_test(), (1, 0));

        let mut one_under = transaction_engine();
        one_under.resource_limits = ResourceLimits {
            max_encoded_state_bytes: encoded_len - 1,
            ..one_under.resource_limits.clone()
        };
        reset_import_state_encoding_counts_for_test();
        let error = match one_under.build_candidate_from_document_in_doc(
            validated_opaque_source(&one_under, &value),
            TransactionOrigin::DocumentImport,
            one_under_doc,
        ) {
            Ok(_) => panic!("one under the authoritative candidate bytes must reject"),
            Err(error) => error,
        };
        assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
        assert_eq!(error.limit, Some(encoded_len - 1));
        assert_eq!(error.actual, Some(encoded_len));
        assert_eq!(take_import_state_encoding_counts_for_test(), (1, 0));
    }

    #[test]
    fn opaque_only_import_defers_replica_then_first_structural_mutation_bootstraps() {
        let opaque = json!({
            "type": "doc",
            "content": [{
                "type": "benchmarkOpaqueBlock",
                "attrs": { "payload": "x".repeat(32 * 1024) }
            }]
        });
        let mut engine = transaction_engine();
        reset_import_state_encoding_counts_for_test();
        reset_import_receipt_state_decodings_for_test();
        reset_import_receipt_sha256_counts_for_test();

        engine
            .import_json(&opaque.to_string(), TransactionOrigin::DocumentImport)
            .unwrap();

        assert_eq!(
            take_import_state_encoding_counts_for_test(),
            (1, 0),
            "candidate admission still performs its one mandatory bounded encode"
        );
        assert_eq!(take_import_receipt_state_decodings_for_test(), 0);
        assert_eq!(take_import_receipt_sha256_counts_for_test(), (0, 0));
        assert!(engine.prepared_candidate_cache.is_none());
        assert!(engine
            .derived_state
            .as_ref()
            .unwrap()
            .mutation_lookup_seed
            .is_unavailable_for_test());
        assert_eq!(engine.document_json().unwrap(), opaque);

        reset_prepared_candidate_cache_counts_for_test();
        reset_encoded_state_reuse_counts_for_test();
        engine
            .apply_typed_transaction(paragraph_insert_transaction(&engine, 70_115))
            .unwrap();

        assert_eq!(take_prepared_candidate_cache_counts_for_test(), (0, 1));
        assert_eq!(take_encoded_state_reuse_counts_for_test(), (0, 1, 0));
        assert!(engine.prepared_candidate_cache.is_some());
        assert!(engine
            .derived_state
            .as_ref()
            .unwrap()
            .mutation_lookup_seed
            .is_ready_for_test());
        let json = engine.document_json().unwrap();
        assert_eq!(json["content"][0], opaque["content"][0]);
        assert_eq!(json["content"][1]["type"], "paragraph");
    }

    #[test]
    fn validated_import_commit_does_not_recompute_schema_fingerprint() {
        use crate::schema::{
            reset_schema_fingerprint_count_for_test, take_schema_fingerprint_count_for_test,
        };

        let mut engine = transaction_engine();
        let candidate = validated_json_import_candidate(&engine);
        reset_schema_fingerprint_count_for_test();

        engine
            .commit_candidate(candidate, TransactionOrigin::DocumentImport)
            .unwrap();

        let total_fingerprints = take_schema_fingerprint_count_for_test();
        assert_eq!(
            total_fingerprints, 1,
            "the test-only render-cache slow invariant remains the sole fingerprint call"
        );
        assert_eq!(
            total_fingerprints.saturating_sub(1),
            0,
            "the exact immutable schema and canonical-artifact seals make commit-time hashing redundant"
        );
    }

    #[test]
    fn import_lookup_schema_seal_drift_falls_back_exactly_once() {
        use crate::yrs_engine::mutation::{
            reset_localized_lookup_counts_for_test, take_localized_lookup_counts_for_test,
        };

        for case in [
            "schemaToken",
            "currentSchemaFingerprint",
            "equalDistinctSchemaPointer",
        ] {
            let engine = transaction_engine();
            let mut candidate = validated_json_import_candidate(&engine);
            let (source_document, canonical_artifact) = match &candidate.state {
                EngineDocumentState::Ready {
                    document,
                    canonical_artifact,
                } => (document.clone(), canonical_artifact.clone()),
                EngineDocumentState::AwaitingRemote => {
                    panic!("validated import candidate must be ready")
                }
            };
            let mut receipt = candidate
                .import_encoded_state_receipt
                .take()
                .expect("validated import candidate carries its lookup receipt");
            if case == "schemaToken" {
                receipt
                    .lookup_materialization
                    .as_mut()
                    .unwrap()
                    .schema_token ^= 1;
            }
            let equal_schema = engine.schema.clone();
            let schema = if case == "equalDistinctSchemaPointer" {
                &equal_schema
            } else {
                &engine.schema
            };
            let drifted_schema_fingerprint = format!("{}-drifted", engine.schema_fingerprint);
            let schema_fingerprint = if case == "currentSchemaFingerprint" {
                drifted_schema_fingerprint.as_str()
            } else {
                engine.schema_fingerprint.as_str()
            };

            reset_localized_lookup_counts_for_test();
            let fused = receipt.take_matching_lookup_materialization(
                &candidate.doc,
                &engine.fragment_name,
                &source_document,
                &canonical_artifact,
                &engine.resource_limits,
                &engine.editing_limits,
                engine.max_length,
                schema,
                schema_fingerprint,
                1,
                1,
            );
            assert!(fused.is_none(), "{case}");

            let txn = candidate.doc.transact();
            let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
            crate::yrs_engine::mutation::MutationLookupSeed::build(
                0,
                &txn,
                &fragment,
                schema,
                &source_document,
                &engine.resource_limits,
                &engine.editing_limits,
                engine.max_length,
                schema_fingerprint,
                1,
                1,
            )
            .unwrap();
            assert_eq!(take_localized_lookup_counts_for_test().0, 1, "{case}");
        }
    }

    fn validated_json_import_candidate(engine: &YrsDocumentEngine) -> CandidateDocument {
        let value = serde_json::json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{"type": "text", "text": "abc"}]
            }]
        });
        let document =
            from_prosemirror_json(&value, &engine.schema, UnknownTypeMode::Preserve).unwrap();
        let source = ValidatedImportDocument::new(
            document,
            &engine.schema,
            &engine.canonical_schema,
            &engine.resource_limits,
            Some(serde_json::to_vec(&value).unwrap().len()),
        )
        .unwrap();
        engine
            .build_candidate_from_document(source, TransactionOrigin::DocumentImport)
            .unwrap()
    }

    fn equal_clock_divergent_valid_update(
        engine: &YrsDocumentEngine,
        candidate: &CandidateDocument,
    ) -> Vec<u8> {
        let divergent = super::equivalent_private_candidate_doc(&candidate.doc);
        let empty_json = json!({
            "type": engine.schema.doc_node_type(),
            "content": [],
        });
        let divergent_json = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{"type": "text", "text": "xyz"}]
            }]
        });
        let codec = super::YrsDocumentCodec::new(&engine.schema, &engine.resource_limits);
        {
            let mut txn =
                divergent.transact_mut_with(TransactionOrigin::DocumentImport.as_yrs_origin());
            let fragment = txn.get_or_insert_xml_fragment(engine.fragment_name.as_str());
            codec
                .apply_json(&fragment, &mut txn, &empty_json, &divergent_json)
                .unwrap();
        }
        let candidate_txn = candidate.doc.transact();
        let divergent_txn = divergent.transact();
        assert_eq!(
            divergent_txn.state_vector(),
            candidate_txn.state_vector(),
            "the tamper must keep identical client clocks"
        );
        let candidate_encoded = candidate_txn.encode_state_as_update_v1(&StateVector::default());
        let divergent_encoded = divergent_txn.encode_state_as_update_v1(&StateVector::default());
        assert_ne!(
            divergent_encoded, candidate_encoded,
            "the tamper must carry different valid content"
        );
        divergent_encoded
    }

    #[test]
    fn tampered_import_encoded_state_receipt_falls_back_to_one_cache_encode() {
        for case in [
            "bytes",
            "sha256",
            "stateVector",
            "fragment",
            "clientId",
            "guid",
            "offsetKind",
            "skipGc",
            "deleteSetEligibility",
            "lookupSourceDocument",
            "lookupCanonicalArtifact",
            "lookupResourceLimits",
            "lookupEditingLimits",
            "lookupMaxLength",
            "lookupSchemaToken",
            "lookupStoreToken",
        ] {
            let lookup_only_tamper = case.starts_with("lookup");
            let mut engine = transaction_engine();
            let mut candidate = validated_json_import_candidate(&engine);
            let installed = engine.derived_state.as_ref().unwrap();
            let foreign_document = installed.document.clone();
            let foreign_artifact = installed.canonical_artifact.clone();
            let receipt = candidate
                .import_encoded_state_receipt
                .as_mut()
                .expect("validated JSON candidates carry one private encoded-state receipt");
            match case {
                "bytes" => receipt.encoded_state = vec![0xff],
                "sha256" => receipt.encoded_state_sha256[0] ^= 1,
                "stateVector" => receipt.state_vector = StateVector::default(),
                "fragment" => receipt.fragment_id = BranchID::Root(Arc::from("foreign")),
                "clientId" => receipt.client_id = ClientID::new(receipt.client_id.get() ^ 1),
                "guid" => receipt.guid = Arc::from("foreign-guid"),
                "offsetKind" => receipt.offset_kind = OffsetKind::Bytes,
                "skipGc" => receipt.skip_gc = !receipt.skip_gc,
                "deleteSetEligibility" => {
                    receipt.delete_set_is_empty = !receipt.delete_set_is_empty
                }
                "lookupSourceDocument" => {
                    receipt
                        .lookup_materialization
                        .as_mut()
                        .unwrap()
                        .source_document = foreign_document
                }
                "lookupCanonicalArtifact" => {
                    receipt
                        .lookup_materialization
                        .as_mut()
                        .unwrap()
                        .canonical_artifact = foreign_artifact
                }
                "lookupResourceLimits" => {
                    receipt
                        .lookup_materialization
                        .as_mut()
                        .unwrap()
                        .resource_limits
                        .max_document_nodes ^= 1
                }
                "lookupEditingLimits" => {
                    receipt
                        .lookup_materialization
                        .as_mut()
                        .unwrap()
                        .editing_limits
                        .max_operations_per_transaction ^= 1
                }
                "lookupMaxLength" => {
                    receipt.lookup_materialization.as_mut().unwrap().max_length = Some(1)
                }
                "lookupSchemaToken" => {
                    receipt
                        .lookup_materialization
                        .as_mut()
                        .unwrap()
                        .schema_token ^= 1
                }
                "lookupStoreToken" => {
                    receipt.lookup_materialization.as_mut().unwrap().store_token ^= 1
                }
                _ => unreachable!(),
            }
            reset_import_state_encoding_counts_for_test();
            crate::yrs_engine::mutation::reset_localized_lookup_counts_for_test();

            engine
                .commit_candidate(candidate, TransactionOrigin::DocumentImport)
                .unwrap();

            assert_eq!(
                take_import_state_encoding_counts_for_test(),
                if lookup_only_tamper { (0, 0) } else { (0, 1) },
                "{case}"
            );
            assert_eq!(
                crate::yrs_engine::mutation::take_localized_lookup_counts_for_test().0,
                1,
                "{case}"
            );
            assert_prepared_candidate_state_vector_exact(&engine);
            assert_eq!(
                engine
                    .prepared_candidate_cache
                    .as_ref()
                    .unwrap()
                    .encoded_state_seal
                    .as_ref()
                    .unwrap()
                    .encoded_state,
                super::encode_state_bounded(&engine.doc, &engine.resource_limits).unwrap(),
                "{case}"
            );
        }
    }

    #[test]
    fn equal_clock_divergent_valid_receipt_bytes_fall_back_to_authoritative_state() {
        let mut engine = transaction_engine();
        let mut candidate = validated_json_import_candidate(&engine);
        let divergent_encoded = equal_clock_divergent_valid_update(&engine, &candidate);
        candidate
            .import_encoded_state_receipt
            .as_mut()
            .unwrap()
            .encoded_state = divergent_encoded.clone();
        reset_import_state_encoding_counts_for_test();

        engine
            .commit_candidate(candidate, TransactionOrigin::DocumentImport)
            .unwrap();

        assert_eq!(take_import_state_encoding_counts_for_test(), (0, 1));
        assert_prepared_candidate_state_vector_exact(&engine);
        let sealed = &engine
            .prepared_candidate_cache
            .as_ref()
            .unwrap()
            .encoded_state_seal
            .as_ref()
            .unwrap()
            .encoded_state;
        assert_eq!(
            sealed,
            &super::encode_state_bounded(&engine.doc, &engine.resource_limits).unwrap()
        );
        assert_ne!(sealed, &divergent_encoded);
    }

    #[test]
    fn oversized_receipt_falls_back_before_standard_update_decode() {
        let engine = transaction_engine();
        let mut candidate = validated_json_import_candidate(&engine);
        let mut receipt = candidate.import_encoded_state_receipt.take().unwrap();
        let limit = receipt.encoded_state.len().checked_mul(2).unwrap();
        receipt.encoded_state = vec![0xff; limit + 1];
        receipt.encoded_state_sha256 = sha2::Sha256::digest(&receipt.encoded_state).into();
        reset_import_state_encoding_counts_for_test();
        reset_import_receipt_state_decodings_for_test();

        let cache = super::prepare_import_candidate_cache(
            &candidate.doc,
            &engine.fragment_name,
            &ResourceLimits {
                max_encoded_state_bytes: limit,
                ..engine.resource_limits.clone()
            },
            Some(receipt),
            None,
            1,
            1,
        );

        assert!(cache.is_some());
        assert_eq!(take_import_state_encoding_counts_for_test(), (0, 1));
        assert_eq!(take_import_receipt_state_decodings_for_test(), 0);
    }

    #[test]
    fn import_receipt_obeys_exact_retained_and_two_x_candidate_boundaries() {
        let prepare_at = |boundary: &str| {
            let engine = transaction_engine();
            let mut candidate = validated_json_import_candidate(&engine);
            let receipt = candidate.import_encoded_state_receipt.take().unwrap();
            let len = receipt.encoded_state.len();
            let retained =
                super::retained_import_state_charge(len, receipt.encoded_state.capacity()).unwrap();
            let limit = match boundary {
                "retained" => retained,
                "oneUnderRetained" => retained - 1,
                "twoX" => len.checked_mul(2).unwrap(),
                _ => unreachable!(),
            };
            reset_import_state_encoding_counts_for_test();
            let cache = super::prepare_import_candidate_cache(
                &candidate.doc,
                &engine.fragment_name,
                &ResourceLimits {
                    max_encoded_state_bytes: limit,
                    ..engine.resource_limits.clone()
                },
                Some(receipt),
                None,
                1,
                1,
            );
            assert_eq!(take_import_state_encoding_counts_for_test(), (0, 0));
            cache
        };
        assert!(prepare_at("retained").unwrap().encoded_state_seal.is_some());
        assert!(prepare_at("oneUnderRetained")
            .unwrap()
            .encoded_state_seal
            .is_none());
        assert!(prepare_at("twoX").unwrap().encoded_state_seal.is_none());
    }

    #[test]
    fn import_encoded_state_seal_obeys_exact_retained_charge_without_dropping_two_x_cache() {
        let mut source = transaction_engine();
        source
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let encoded = super::encode_state_bounded(&source.doc, &source.resource_limits).unwrap();
        let encoded_len = encoded.len();
        let encoded_capacity = encoded.capacity();
        let exact_retained_charge =
            super::retained_import_state_charge(encoded_len, encoded_capacity).unwrap();

        let exact_cache = super::prepare_import_candidate_cache(
            &source.doc,
            &source.fragment_name,
            &ResourceLimits {
                max_encoded_state_bytes: exact_retained_charge,
                ..source.resource_limits.clone()
            },
            None,
            None,
            source.revision,
            source.yrs_state_epoch,
        )
        .expect("the exact retained charge retains the private candidate");
        let exact_seal = exact_cache.encoded_state_seal.as_ref().unwrap();
        assert_eq!(exact_seal.encoded_state.len(), encoded_len);
        assert_eq!(exact_seal.encoded_state.capacity(), encoded_capacity);

        let one_under_cache = super::prepare_import_candidate_cache(
            &source.doc,
            &source.fragment_name,
            &ResourceLimits {
                max_encoded_state_bytes: exact_retained_charge - 1,
                ..source.resource_limits.clone()
            },
            None,
            None,
            source.revision,
            source.yrs_state_epoch,
        )
        .expect("a document above one third but within the 2x ceiling retains its candidate");
        assert!(one_under_cache.encoded_state_seal.is_none());

        let exact_two_x_cache = super::prepare_import_candidate_cache(
            &source.doc,
            &source.fragment_name,
            &ResourceLimits {
                max_encoded_state_bytes: encoded_len.checked_mul(2).unwrap(),
                ..source.resource_limits.clone()
            },
            None,
            None,
            source.revision,
            source.yrs_state_epoch,
        )
        .expect("the existing exact 2x candidate admission remains unchanged");
        assert!(exact_two_x_cache.encoded_state_seal.is_none());
    }

    fn assert_next_insert_uses_full_current_state_encode(
        engine: &mut YrsDocumentEngine,
        request_id: u64,
    ) {
        reset_encoded_state_reuse_counts_for_test();
        engine
            .apply_typed_transaction(insert_transaction(engine, request_id))
            .unwrap();
        assert_eq!(take_encoded_state_reuse_counts_for_test(), (0, 1, 0));
    }

    fn imported_engine_with_sealed_state() -> YrsDocumentEngine {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        assert!(engine
            .prepared_candidate_cache
            .as_ref()
            .and_then(|cache| cache.encoded_state_seal.as_ref())
            .is_some());
        engine
    }

    #[test]
    fn sealed_state_vector_drift_falls_back() {
        let mut engine = imported_engine_with_sealed_state();
        let compiled = engine
            .compile_typed_transaction(insert_transaction(&engine, 70_115))
            .unwrap();
        let live_doc = engine.doc.clone();
        let live_txn = live_doc.transact();
        let live_fragment = live_txn
            .get_xml_fragment(engine.fragment_name.as_str())
            .unwrap();
        let exact_state_vector = live_txn.state_vector();
        engine
            .prepared_candidate_cache
            .as_mut()
            .unwrap()
            .state_vector = StateVector::default();
        let reused = engine
            .prepared_candidate_cache
            .as_mut()
            .unwrap()
            .take_matching_encoded_state(
                &live_doc,
                &live_fragment,
                &compiled.mutation_plan,
                engine.revision,
                engine.yrs_state_epoch,
                engine.resource_limits.max_encoded_state_bytes,
            );
        assert!(reused.is_none());
        engine
            .prepared_candidate_cache
            .as_mut()
            .unwrap()
            .state_vector = exact_state_vector;
        drop(live_txn);

        reset_encoded_state_reuse_counts_for_test();
        engine.apply_compiled_transaction(compiled, true).unwrap();
        assert_eq!(take_encoded_state_reuse_counts_for_test(), (0, 1, 0));
    }

    #[test]
    fn import_with_nonempty_delete_set_retains_candidate_without_sealed_bytes() {
        let mut source = imported_engine_with_sealed_state();
        let from = RevisionedPosition {
            offset: 1,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::After,
        };
        let to = RevisionedPosition { offset: 2, ..from };
        source
            .apply_typed_transaction(TypedTransaction {
                request_id: 70_116,
                base_document_revision: source.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: vec![TypedOperation::DeleteRange {
                    range: RevisionedRange { from, to },
                }],
                selection_intent: SelectionIntent::Preserve,
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap();
        assert!(!source.doc.transact().snapshot().delete_set.is_empty());

        let cache = super::prepare_import_candidate_cache(
            &source.doc,
            &source.fragment_name,
            &source.resource_limits,
            None,
            None,
            source.revision,
            source.yrs_state_epoch,
        )
        .expect("the existing 2x private candidate remains available");
        assert!(cache.encoded_state_seal.is_none());
    }

    #[test]
    fn sealed_state_fragment_options_revision_and_epoch_drift_fall_back() {
        let mut stale_fragment = imported_engine_with_sealed_state();
        stale_fragment
            .prepared_candidate_cache
            .as_mut()
            .unwrap()
            .encoded_state_seal
            .as_mut()
            .unwrap()
            .fragment_id = BranchID::Root(Arc::from("other"));
        assert_next_insert_uses_full_current_state_encode(&mut stale_fragment, 70_118);

        let mut stale_options = imported_engine_with_sealed_state();
        let seal = stale_options
            .prepared_candidate_cache
            .as_mut()
            .unwrap()
            .encoded_state_seal
            .as_mut()
            .unwrap();
        seal.offset_kind = match seal.offset_kind {
            OffsetKind::Bytes => OffsetKind::Utf16,
            OffsetKind::Utf16 => OffsetKind::Bytes,
        };
        assert_next_insert_uses_full_current_state_encode(&mut stale_options, 70_119);

        let mut stale_revision = imported_engine_with_sealed_state();
        stale_revision
            .prepared_candidate_cache
            .as_mut()
            .unwrap()
            .encoded_state_seal
            .as_mut()
            .unwrap()
            .document_revision = stale_revision.revision.saturating_add(1);
        assert_next_insert_uses_full_current_state_encode(&mut stale_revision, 70_120);

        let mut stale_epoch = imported_engine_with_sealed_state();
        stale_epoch
            .prepared_candidate_cache
            .as_mut()
            .unwrap()
            .encoded_state_seal
            .as_mut()
            .unwrap()
            .yrs_state_epoch = stale_epoch.yrs_state_epoch.saturating_add(1);
        assert_next_insert_uses_full_current_state_encode(&mut stale_epoch, 70_121);
    }

    #[test]
    fn sealed_state_rechecks_current_limit_and_survives_selection_only_state_change() {
        let mut limit_drift = transaction_engine();
        let large_source = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{"type": "text", "text": "a".repeat(2_048)}]
            }]
        })
        .to_string();
        limit_drift
            .import_json(&large_source, TransactionOrigin::DocumentImport)
            .unwrap();
        let retained_len = limit_drift
            .prepared_candidate_cache
            .as_ref()
            .unwrap()
            .encoded_state_seal
            .as_ref()
            .unwrap()
            .encoded_state
            .len();
        limit_drift.resource_limits.max_encoded_state_bytes =
            retained_len.checked_mul(3).unwrap() - 1;
        assert_next_insert_uses_full_current_state_encode(&mut limit_drift, 70_122);

        let mut selection_only = imported_engine_with_sealed_state();
        let document_revision = selection_only.revision;
        let yrs_state_epoch = selection_only.yrs_state_epoch;
        select_text(&mut selection_only, 70_123, 1, 3);
        assert_eq!(selection_only.revision, document_revision);
        assert_eq!(selection_only.yrs_state_epoch, yrs_state_epoch);
        assert!(selection_only
            .prepared_candidate_cache
            .as_ref()
            .unwrap()
            .encoded_state_seal
            .is_some());
        reset_encoded_state_reuse_counts_for_test();
        selection_only
            .apply_typed_transaction(insert_transaction(&selection_only, 70_124))
            .unwrap();
        assert_eq!(take_encoded_state_reuse_counts_for_test(), (0, 0, 1));
    }

    #[test]
    fn sealed_state_bytes_match_stock_oracle_with_history_undo_redo_parity() {
        let mut optimized = imported_engine_with_sealed_state();
        let mut stock = imported_engine_with_sealed_state();
        stock
            .prepared_candidate_cache
            .as_mut()
            .unwrap()
            .encoded_state_seal = None;
        let stock_current =
            super::encode_state_bounded(&optimized.doc, &optimized.resource_limits).unwrap();
        assert_eq!(
            optimized
                .prepared_candidate_cache
                .as_ref()
                .unwrap()
                .encoded_state_seal
                .as_ref()
                .unwrap()
                .encoded_state
                .as_slice(),
            stock_current.as_slice()
        );

        optimized
            .apply_typed_transaction(insert_transaction(&optimized, 70_125))
            .unwrap();
        stock
            .apply_typed_transaction(insert_transaction(&stock, 70_125))
            .unwrap();
        assert_eq!(optimized.document_json(), stock.document_json());
        assert_eq!(optimized.can_undo(), stock.can_undo());
        assert_eq!(optimized.can_redo(), stock.can_redo());

        optimized.undo(70_126).unwrap();
        stock.undo(70_126).unwrap();
        assert_eq!(optimized.document_json(), stock.document_json());
        assert_eq!(optimized.can_redo(), stock.can_redo());

        optimized.redo(70_127).unwrap();
        stock.redo(70_127).unwrap();
        assert_eq!(optimized.document_json(), stock.document_json());
        assert_eq!(optimized.can_undo(), stock.can_undo());
    }

    #[test]
    fn prepared_candidate_seals_actual_clock_for_redundant_inherited_mark_insert() {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"ab"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let local_client = engine.doc.client_id();

        let first = engine
            .compile_typed_transaction(marked_insert_transaction(&engine, 70_109, "a"))
            .unwrap();
        assert_eq!(first.authored_clock_units, 3);
        let before_first = engine.doc.transact().state_vector().get(&local_client);
        engine.apply_compiled_transaction(first, true).unwrap();
        let after_first = engine.doc.transact().state_vector().get(&local_client);
        assert_eq!(after_first - before_first, 3);

        let second = engine
            .compile_typed_transaction(marked_insert_transaction(&engine, 70_110, "b"))
            .unwrap();
        assert_eq!(second.authored_clock_units, 3);
        let before_second = engine.doc.transact().state_vector().get(&local_client);
        engine.apply_compiled_transaction(second, true).unwrap();
        let after_second = engine.doc.transact().state_vector().get(&local_client);

        assert_eq!(after_second - before_second, 1);
        assert_prepared_candidate_state_vector_exact(&engine);
    }

    #[test]
    fn prepared_candidate_bounds_inherited_format_suspension_at_text_boundaries() {
        struct Case {
            name: &'static str,
            source: &'static str,
            offset: u32,
            inserted: &'static str,
            marks: Vec<Mark>,
            expected_bound: u64,
        }

        let bold = || Mark::new("bold".into(), HashMap::new());
        let italic = || Mark::new("italic".into(), HashMap::new());
        let cases = [
            Case {
                name: "plain at start",
                source: "ab",
                offset: 0,
                inserted: "x",
                marks: vec![],
                expected_bound: 3,
            },
            Case {
                name: "plain inside",
                source: "ab",
                offset: 1,
                inserted: "x",
                marks: vec![],
                expected_bound: 3,
            },
            Case {
                name: "plain at end",
                source: "ab",
                offset: 2,
                inserted: "x",
                marks: vec![],
                expected_bound: 3,
            },
            Case {
                name: "same mark inside",
                source: "ab",
                offset: 1,
                inserted: "x",
                marks: vec![bold()],
                expected_bound: 3,
            },
            Case {
                name: "different mark inside",
                source: "ab",
                offset: 1,
                inserted: "x",
                marks: vec![italic()],
                expected_bound: 5,
            },
            Case {
                name: "plain unicode inside",
                source: "😀b",
                offset: 1,
                inserted: "🦀",
                marks: vec![],
                expected_bound: 4,
            },
        ];

        for (index, case) in cases.into_iter().enumerate() {
            let mut engine = transaction_engine();
            engine
                .import_json(
                    &serde_json::json!({
                        "type": "doc",
                        "content": [{
                            "type": "paragraph",
                            "content": [{
                                "type": "text",
                                "text": case.source,
                                "marks": [{ "type": "bold" }]
                            }]
                        }]
                    })
                    .to_string(),
                    TransactionOrigin::DocumentImport,
                )
                .unwrap();
            let request_id = 70_120 + u64::try_from(index).unwrap();
            let compiled = engine
                .compile_typed_transaction(TypedTransaction {
                    request_id,
                    base_document_revision: engine.revision(),
                    origin: TransactionOrigin::LocalInput,
                    operations: vec![TypedOperation::InsertText {
                        at: RevisionedPosition {
                            offset: case.offset,
                            kind: EditorOffsetKind::Scalar,
                            affinity: Affinity::After,
                        },
                        text: case.inserted.into(),
                        marks: case.marks,
                    }],
                    selection_intent: SelectionIntent::UseOperationResult,
                    history_policy: HistoryPolicy::Skip,
                })
                .unwrap();
            assert_eq!(
                compiled.authored_clock_units, case.expected_bound,
                "{}",
                case.name
            );
            let local_client = engine.doc.client_id();
            let before = engine.doc.transact().state_vector().get(&local_client);
            engine.apply_compiled_transaction(compiled, true).unwrap();
            let after = engine.doc.transact().state_vector().get(&local_client);
            assert!(
                u64::from(after - before) <= case.expected_bound,
                "{}",
                case.name
            );
            assert_prepared_candidate_state_vector_exact(&engine);
        }

        let mut boundary = transaction_engine();
        boundary
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"a","marks":[{"type":"bold"}]},{"type":"text","text":"b","marks":[{"type":"italic"}]}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let compiled = boundary
            .compile_typed_transaction(TypedTransaction {
                request_id: 70_126,
                base_document_revision: boundary.revision(),
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: RevisionedPosition {
                        offset: 1,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    text: "x".into(),
                    marks: vec![],
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap();
        // The lowering selects one exact storage target at this semantic
        // boundary; only that target's touching bold run contributes.
        assert_eq!(compiled.authored_clock_units, 3);
        boundary.apply_compiled_transaction(compiled, true).unwrap();
        assert_prepared_candidate_state_vector_exact(&boundary);

        let mut delete_then_insert = transaction_engine();
        delete_then_insert
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"ab","marks":[{"type":"bold"}]}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let compiled = delete_then_insert
            .compile_typed_transaction(TypedTransaction {
                request_id: 70_127,
                base_document_revision: delete_then_insert.revision(),
                origin: TransactionOrigin::LocalInput,
                operations: vec![
                    TypedOperation::DeleteRange {
                        range: RevisionedRange {
                            from: RevisionedPosition {
                                offset: 0,
                                kind: EditorOffsetKind::Scalar,
                                affinity: Affinity::After,
                            },
                            to: RevisionedPosition {
                                offset: 2,
                                kind: EditorOffsetKind::Scalar,
                                affinity: Affinity::Before,
                            },
                        },
                    },
                    TypedOperation::InsertText {
                        at: RevisionedPosition {
                            offset: 0,
                            kind: EditorOffsetKind::Scalar,
                            affinity: Affinity::After,
                        },
                        text: "x".into(),
                        marks: vec![],
                    },
                ],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap();
        assert_eq!(compiled.authored_clock_units, 3);
        delete_then_insert
            .apply_compiled_transaction(compiled, true)
            .unwrap();
        assert_prepared_candidate_state_vector_exact(&delete_then_insert);
    }

    #[test]
    fn prepared_candidate_cache_failure_is_private_atomic_and_falls_back_once() {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let before = atomic_audit(&engine);
        set_compiled_commit_stage_failpoint_for_test(Some(
            CompiledCommitPreparationStage::HistoryUpdateEncoding,
        ));
        reset_encoded_state_reuse_counts_for_test();

        let error = engine
            .apply_typed_transaction(insert_transaction(&engine, 70_105))
            .expect_err("candidate encoding failpoint must reject before the live write");

        set_compiled_commit_stage_failpoint_for_test(None);
        assert!(error.message.contains("historyUpdateEncoding"));
        assert_eq!(atomic_audit(&engine), before);
        assert!(engine.prepared_candidate_cache.is_none());
        assert_eq!(take_encoded_state_reuse_counts_for_test(), (0, 0, 1));
        reset_prepared_candidate_cache_counts_for_test();
        reset_encoded_state_reuse_counts_for_test();

        engine
            .apply_typed_transaction(insert_transaction(&engine, 70_106))
            .unwrap();

        assert!(engine.prepared_candidate_cache.is_some());
        assert_eq!(take_prepared_candidate_cache_counts_for_test(), (0, 1));
        assert_eq!(take_encoded_state_reuse_counts_for_test(), (0, 1, 0));
    }

    #[test]
    fn prepared_candidate_cache_revalidates_stale_revision_seal_before_reuse() {
        use crate::yrs_engine::mutation::{
            reset_localized_lookup_counts_for_test, take_localized_lookup_counts_for_test,
        };

        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        engine
            .prepared_candidate_cache
            .as_mut()
            .unwrap()
            .document_revision = engine.revision.saturating_add(1);
        reset_prepared_candidate_cache_counts_for_test();
        reset_localized_lookup_counts_for_test();

        engine
            .apply_typed_transaction(insert_transaction(&engine, 70_108))
            .unwrap();
        let cache_counts = take_prepared_candidate_cache_counts_for_test();
        let lookup_counts = take_localized_lookup_counts_for_test();
        let cached_encoded = super::encode_state_bounded(
            &engine.prepared_candidate_cache.as_ref().unwrap().doc,
            &engine.resource_limits,
        )
        .unwrap();

        assert_eq!(cache_counts, (0, 1));
        assert_eq!(lookup_counts, (1, 1, 1));
        assert_eq!(
            engine.document_json().unwrap()["content"][0]["content"][0]["text"],
            "axbc"
        );
        assert_eq!(cached_encoded, engine.encoded_state().unwrap());
    }

    #[test]
    fn imported_candidate_cache_supplies_first_staged_lookup_without_live_rebuild() {
        use crate::yrs_engine::mutation::{
            reset_localized_lookup_counts_for_test, take_localized_lookup_counts_for_test,
        };

        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        assert!(engine
            .derived_state
            .as_ref()
            .unwrap()
            .mutation_lookup_seed
            .is_unavailable_for_test());
        reset_localized_lookup_counts_for_test();

        engine
            .apply_command(70_107, TypedCommand::InsertText { text: "x".into() })
            .unwrap()
            .unwrap();

        assert_eq!(take_localized_lookup_counts_for_test(), (0, 1, 1));
    }

    #[test]
    fn validated_import_materializes_ready_lookup_without_a_second_tree_scan() {
        use crate::yrs_engine::mutation::{
            reset_localized_lookup_counts_for_test, take_localized_lookup_counts_for_test,
        };

        let mut engine = transaction_engine();
        reset_localized_lookup_counts_for_test();

        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();

        assert_eq!(
            take_localized_lookup_counts_for_test(),
            (0, 0, 0),
            "validated codec traversal must carry the exact ready lookup payload"
        );
        assert!(engine
            .prepared_candidate_cache
            .as_ref()
            .and_then(|cache| cache.staged_lookup_seed.as_ref())
            .is_some());
    }

    #[test]
    fn validated_import_lookup_materialization_matches_the_ordinary_builder() {
        let inputs = [
            r#"{"type":"doc","content":[{"type":"heading","attrs":{"level":2}}]}"#,
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"plain"},{"type":"text","text":" bold","marks":[{"type":"bold"}]},{"type":"text","text":" 🦀"}]}]}"#,
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"hardBreak"},{"type":"text","text":"middle"},{"type":"hardBreak"},{"type":"hardBreak"}]}]}"#,
            r#"{"type":"doc","content":[{"type":"blockquote","content":[{"type":"heading","attrs":{"level":2},"content":[{"type":"text","text":"nested"}]}]},{"type":"horizontal_rule"},{"type":"mystery_widget","attrs":{"payload":{"x":[1,true,"v"]}}}]}"#,
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"a"},{"type":"text","text":"b","marks":[{"type":"italic"}]},{"type":"text","text":"c"}]}]}"#,
        ];

        for input in inputs {
            let mut engine = transaction_engine();
            engine
                .import_json(input, TransactionOrigin::DocumentImport)
                .unwrap();
            let staged = engine
                .prepared_candidate_cache
                .as_ref()
                .and_then(|cache| cache.staged_lookup_seed.as_ref())
                .unwrap_or_else(|| {
                    panic!("validated import carries the fused ready seed: {input}")
                });
            let txn = engine.doc.transact();
            let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
            let state = engine.derived_state.as_ref().unwrap();
            assert!(
                crate::yrs_engine::mutation::lookup_payload_legacy_parity_for_test(
                    &txn,
                    &fragment,
                    &engine.schema,
                ),
                "{input}"
            );
            let ordinary = crate::yrs_engine::mutation::MutationLookupSeed::build(
                77_001,
                &txn,
                &fragment,
                &engine.schema,
                &state.document,
                &engine.resource_limits,
                &engine.editing_limits,
                engine.max_length,
                &engine.schema_fingerprint,
                engine.yrs_state_epoch,
                engine.revision,
            )
            .unwrap();
            assert!(staged.has_same_ready_payload_for_test(&ordinary), "{input}");
        }
    }

    #[test]
    fn lookup_materialization_matches_legacy_for_nested_fragment_and_empty_text_storage() {
        let engine = transaction_engine();
        let doc = utf16_doc();
        let mut txn = doc.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("content");
        let nested = XmlFragmentPrelim::new::<_, XmlIn>([
            XmlIn::from(XmlTextPrelim::new("")),
            XmlIn::from(XmlTextPrelim::new("x")),
        ]);
        fragment.insert(&mut txn, 0, XmlIn::from(nested));
        drop(txn);
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("content").unwrap();

        assert!(
            crate::yrs_engine::mutation::lookup_payload_legacy_parity_for_test(
                &txn,
                &fragment,
                &engine.schema,
            )
        );
    }

    #[test]
    fn import_lookup_materialization_failpoints_are_opportunistic_and_fallback_once() {
        use crate::yrs_engine::mutation::{
            reset_localized_lookup_counts_for_test, set_lookup_seed_hydration_failpoint_for_test,
            take_localized_lookup_counts_for_test, LookupSeedHydrationFailpoint,
        };

        for failpoint in [
            LookupSeedHydrationFailpoint::InitialReservation,
            LookupSeedHydrationFailpoint::MapGrowth,
            LookupSeedHydrationFailpoint::MapPublication,
            LookupSeedHydrationFailpoint::BindingPublication,
            LookupSeedHydrationFailpoint::SeedPublication,
        ] {
            let mut engine = transaction_engine();
            reset_localized_lookup_counts_for_test();
            set_lookup_seed_hydration_failpoint_for_test(Some(failpoint));
            engine
                .import_json(
                    r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                    TransactionOrigin::DocumentImport,
                )
                .unwrap();
            set_lookup_seed_hydration_failpoint_for_test(None);
            assert_eq!(
                take_localized_lookup_counts_for_test().0,
                1,
                "{failpoint:?}"
            );

            reset_localized_lookup_counts_for_test();
            engine
                .apply_typed_transaction(insert_transaction(&engine, 77_100))
                .unwrap();
            assert_eq!(
                engine.document_json().unwrap()["content"][0]["content"][0]["text"],
                "axbc",
                "{failpoint:?}"
            );
            assert_prepared_candidate_state_vector_exact(&engine);
        }
    }

    #[test]
    fn ordinary_lookup_collection_fails_fast_while_codec_projection_finishes() {
        use crate::yrs_engine::mutation::{
            reset_import_lookup_event_count_for_test, set_lookup_seed_hydration_failpoint_for_test,
            take_import_lookup_event_count_for_test, LookupSeedHydrationFailpoint,
        };

        let value = json!({
            "type": "doc",
            "content": [
                {"type": "paragraph", "content": [{"type": "text", "text": "first"}]},
                {"type": "paragraph", "content": [{"type": "text", "text": "second"}]}
            ]
        });
        let mut engine = transaction_engine();
        engine
            .import_json(&value.to_string(), TransactionOrigin::DocumentImport)
            .unwrap();
        let txn = engine.doc.transact();
        let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
        let state = engine.derived_state.as_ref().unwrap();
        reset_import_lookup_event_count_for_test();
        set_lookup_seed_hydration_failpoint_for_test(Some(LookupSeedHydrationFailpoint::MapGrowth));
        let error = crate::yrs_engine::mutation::MutationLookupSeed::build(
            77_200,
            &txn,
            &fragment,
            &engine.schema,
            &state.document,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            &engine.schema_fingerprint,
            engine.yrs_state_epoch,
            engine.revision,
        )
        .unwrap_err();
        assert_eq!(error.code, "OPERATION_RESOURCE_EXHAUSTED");
        assert_eq!(take_import_lookup_event_count_for_test(), 2);
        drop(txn);

        let document =
            from_prosemirror_json(&value, &engine.schema, UnknownTypeMode::Preserve).unwrap();
        let source = ValidatedImportDocument::new(
            document,
            &engine.schema,
            &engine.canonical_schema,
            &engine.resource_limits,
            Some(value.to_string().len()),
        )
        .unwrap();
        reset_import_lookup_event_count_for_test();
        let candidate = engine
            .build_candidate_from_document(source, TransactionOrigin::DocumentImport)
            .unwrap();
        set_lookup_seed_hydration_failpoint_for_test(None);
        assert_eq!(take_import_lookup_event_count_for_test(), 2);
        assert!(candidate
            .import_encoded_state_receipt
            .as_ref()
            .is_some_and(|receipt| receipt.lookup_materialization.is_none()));
        let EngineDocumentState::Ready { document, .. } = candidate.state else {
            panic!("validated candidate must be ready")
        };
        assert_eq!(document.root().content().unwrap().child_count(), 2);
    }

    #[test]
    fn missing_text_fallback_rebuilds_once_then_next_insert_localizes() {
        use crate::yrs_engine::mutation::{
            reset_localized_lookup_counts_for_test, take_localized_lookup_counts_for_test,
        };

        let mut engine = transaction_engine();
        reset_localized_lookup_counts_for_test();
        engine
            .apply_command(70_111, TypedCommand::InsertText { text: "x".into() })
            .unwrap()
            .expect("empty paragraph insert must apply");
        engine
            .apply_command(70_112, TypedCommand::InsertText { text: "y".into() })
            .unwrap()
            .expect("existing text insert must apply");

        assert_eq!(take_localized_lookup_counts_for_test(), (1, 1, 1));
    }

    #[test]
    fn selection_only_change_retains_document_scoped_lookup_seed() {
        let mut engine = transaction_engine();
        let before = engine
            .derived_state
            .as_ref()
            .unwrap()
            .mutation_lookup_seed
            .clone();
        let canonical_before = engine
            .derived_state
            .as_ref()
            .unwrap()
            .canonical_artifact
            .clone();
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 70_113,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: vec![],
                selection_intent: SelectionIntent::Set(SelectionInput::All),
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap();
        let after = &engine.derived_state.as_ref().unwrap().mutation_lookup_seed;
        assert!(Arc::ptr_eq(&before, after));
        assert!(canonical_before.ptr_eq(&engine.derived_state.as_ref().unwrap().canonical_artifact));
    }

    #[test]
    fn localized_root_invalidation_rebuilds_ready_once_then_localizes() {
        use crate::yrs_engine::mutation::{
            reset_localized_lookup_counts_for_test, take_localized_lookup_counts_for_test,
        };

        let mut engine = transaction_engine();
        engine
            .apply_command(
                70_113_100,
                TypedCommand::WrapInList {
                    list_type: "bulletList".into(),
                    item_type: "listItem".into(),
                },
            )
            .unwrap()
            .unwrap();
        assert_prepared_candidate_state_vector_exact(&engine);
        let unavailable = engine
            .derived_state
            .as_ref()
            .unwrap()
            .mutation_lookup_seed
            .clone();
        assert!(unavailable.is_unavailable_for_test());

        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 70_113_101,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: vec![],
                selection_intent: SelectionIntent::Preserve,
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap();
        assert!(Arc::ptr_eq(
            &unavailable,
            &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
        ));
        assert!(engine
            .derived_state
            .as_ref()
            .unwrap()
            .mutation_lookup_seed
            .is_unavailable_for_test());

        reset_localized_lookup_counts_for_test();
        engine
            .apply_command(70_113_102, TypedCommand::InsertText { text: "x".into() })
            .unwrap()
            .unwrap();
        assert_eq!(take_localized_lookup_counts_for_test(), (2, 0, 0));
        assert!(engine
            .derived_state
            .as_ref()
            .unwrap()
            .mutation_lookup_seed
            .is_ready_for_test());

        reset_localized_lookup_counts_for_test();
        engine
            .apply_command(70_113_103, TypedCommand::InsertText { text: "y".into() })
            .unwrap()
            .unwrap();
        assert_eq!(take_localized_lookup_counts_for_test(), (0, 1, 1));
        assert!(engine
            .derived_state
            .as_ref()
            .unwrap()
            .mutation_lookup_seed
            .is_ready_for_test());
    }

    #[test]
    fn canonical_artifact_derives_once_per_changed_intermediate_and_never_for_cached_noops() {
        use crate::yrs_engine::canonical::{
            reset_canonical_artifact_counts_for_test, take_canonical_artifact_counts_for_test,
        };

        let mut engine = transaction_engine();
        reset_canonical_artifact_counts_for_test();
        engine
            .apply_typed_transaction(insert_transaction(&engine, 70_114))
            .unwrap();
        assert_eq!(take_canonical_artifact_counts_for_test(), (1, 2));

        let revision = engine.revision();
        reset_canonical_artifact_counts_for_test();
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 70_115,
                base_document_revision: revision,
                origin: TransactionOrigin::LocalApi,
                operations: vec![
                    TypedOperation::InsertText {
                        at: RevisionedPosition {
                            offset: 1,
                            kind: EditorOffsetKind::Scalar,
                            affinity: Affinity::After,
                        },
                        text: "a".into(),
                        marks: vec![],
                    },
                    TypedOperation::InsertText {
                        at: RevisionedPosition {
                            offset: 1,
                            kind: EditorOffsetKind::Scalar,
                            affinity: Affinity::After,
                        },
                        text: "b".into(),
                        marks: vec![],
                    },
                ],
                selection_intent: SelectionIntent::Preserve,
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap();
        assert_eq!(take_canonical_artifact_counts_for_test(), (2, 3));

        reset_canonical_artifact_counts_for_test();
        let commit = engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 70_116,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: vec![TypedOperation::DeleteRange {
                    range: RevisionedRange {
                        from: RevisionedPosition {
                            offset: 1,
                            kind: EditorOffsetKind::Scalar,
                            affinity: Affinity::After,
                        },
                        to: RevisionedPosition {
                            offset: 1,
                            kind: EditorOffsetKind::Scalar,
                            affinity: Affinity::After,
                        },
                    },
                }],
                selection_intent: SelectionIntent::Preserve,
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap();
        assert!(!commit.changed);
        assert_eq!(take_canonical_artifact_counts_for_test(), (0, 0));
    }

    #[test]
    fn public_history_pop_installs_candidate_seed_without_next_edit_rebuild() {
        use crate::yrs_engine::mutation::{
            reset_localized_lookup_counts_for_test, take_localized_lookup_counts_for_test,
        };
        use crate::yrs_engine::observability::{
            reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
        };

        let mut engine = transaction_engine();
        reset_prepared_admission_counts_for_test();
        assert!(engine.undo(70_119).unwrap().is_none());
        assert!(engine.redo(70_120).unwrap().is_none());
        let empty = take_prepared_admission_counts_for_test();
        assert_eq!(empty.staged_seed_preparations, 0);
        assert_eq!(empty.installed_base_seed_publications, 0);
        reset_localized_lookup_counts_for_test();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        assert_eq!(take_localized_lookup_counts_for_test(), (0, 0, 0));

        engine
            .apply_command(70_121, TypedCommand::InsertText { text: "x".into() })
            .unwrap()
            .expect("history insert must apply");
        reset_localized_lookup_counts_for_test();
        reset_prepared_admission_counts_for_test();
        let before_undo = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
        assert!(engine.undo(70_122).unwrap().is_some());
        assert_eq!(take_localized_lookup_counts_for_test(), (1, 0, 0));
        let undo_counts = take_prepared_admission_counts_for_test();
        assert_eq!(undo_counts.staged_seed_preparations, 1);
        assert_eq!(undo_counts.installed_base_seed_publications, 0);
        assert!(!Arc::ptr_eq(
            &before_undo,
            &engine.derived_state.as_ref().unwrap().mutation_lookup_seed,
        ));
        assert!(engine
            .derived_state
            .as_ref()
            .unwrap()
            .mutation_lookup_seed
            .is_ready_for_test());
        reset_localized_lookup_counts_for_test();
        reset_prepared_admission_counts_for_test();
        assert!(engine.redo(70_123).unwrap().is_some());
        assert_eq!(take_localized_lookup_counts_for_test(), (1, 0, 0));
        let redo_counts = take_prepared_admission_counts_for_test();
        assert_eq!(redo_counts.staged_seed_preparations, 1);
        assert_eq!(redo_counts.installed_base_seed_publications, 0);
        assert!(engine
            .derived_state
            .as_ref()
            .unwrap()
            .mutation_lookup_seed
            .is_ready_for_test());

        reset_localized_lookup_counts_for_test();
        engine
            .apply_command(70_124, TypedCommand::InsertText { text: "y".into() })
            .unwrap()
            .expect("the first edit after history restoration must apply");
        assert_eq!(take_localized_lookup_counts_for_test(), (0, 1, 1));

        reset_localized_lookup_counts_for_test();
        engine
            .apply_command(70_125, TypedCommand::InsertText { text: "z".into() })
            .unwrap()
            .expect("the second edit after history restoration must apply");
        assert_eq!(take_localized_lookup_counts_for_test(), (0, 1, 1));

        let mut source = transaction_engine();
        source
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"snapshot"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let snapshot = source.export_snapshot().unwrap();
        reset_localized_lookup_counts_for_test();
        engine.restore_snapshot(&snapshot).unwrap();
        assert_eq!(take_localized_lookup_counts_for_test(), (1, 0, 0));
    }

    #[test]
    fn accepted_remote_candidate_builds_lookup_seed_in_its_own_store() {
        use crate::yrs_engine::mutation::{
            reset_localized_lookup_counts_for_test, take_localized_lookup_counts_for_test,
        };

        let mut source = transaction_engine();
        source
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"remote"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let update = source.encoded_state().unwrap();
        let mut target = YrsDocumentEngine::new(YrsEngineConfig {
            schema: tiptap_schema(),
            fragment_name: "prosemirror".into(),
            initialization_mode: crate::yrs_engine::InitializationMode::AwaitRemote,
            resource_limits: ResourceLimits::default(),
            editing_limits: crate::yrs_engine::EditingLimits::default(),
            max_length: None,
            scope: Some(crate::yrs_engine::DocumentScope {
                document_id: "doc".into(),
                lineage_id: "lineage".into(),
            }),
        })
        .unwrap();
        reset_localized_lookup_counts_for_test();

        let commit = target.apply_remote_update_v1(70_131, &update).unwrap();
        assert!(commit.changed);
        assert_eq!(take_localized_lookup_counts_for_test(), (1, 0, 0));
        reset_localized_lookup_counts_for_test();
        target
            .apply_command(70_132, TypedCommand::InsertText { text: "!".into() })
            .unwrap()
            .expect("remote existing text must accept a local insert");
        assert_prepared_candidate_state_vector_exact(&target);
        let live_vector = target.doc.transact().state_vector();
        assert!(live_vector.get(&ClientID::new(source.client_id())) > 0);
        assert!(live_vector.get(&ClientID::new(target.client_id())) > 0);
        assert_eq!(take_localized_lookup_counts_for_test(), (0, 1, 1));
    }

    #[test]
    fn arbitrary_remote_candidate_rebuilds_revision_bound_render_cache_once() {
        let mut source = transaction_engine();
        source
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"remote"}]},{"type":"paragraph","content":[{"type":"text","text":"tail"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let mut target = YrsDocumentEngine::new(YrsEngineConfig {
            schema: tiptap_schema(),
            fragment_name: "prosemirror".into(),
            initialization_mode: crate::yrs_engine::InitializationMode::AwaitRemote,
            resource_limits: ResourceLimits::default(),
            editing_limits: crate::yrs_engine::EditingLimits::default(),
            max_length: None,
            scope: Some(crate::yrs_engine::DocumentScope {
                document_id: "doc".into(),
                lineage_id: "lineage".into(),
            }),
        })
        .unwrap();
        target
            .apply_remote_update_v1(70_133, &source.encoded_state().unwrap())
            .unwrap();
        source
            .apply_typed_transaction(insert_transaction(&source, 70_134))
            .unwrap();
        let target_vector = target.doc.transact().state_vector();
        let delta = source
            .doc
            .transact()
            .encode_state_as_update_v1(&target_vector);

        crate::render::incremental::reset_cached_render_counts_for_test();
        let commit = target.apply_remote_update_v1(70_135, &delta).unwrap();
        assert!(commit.changed);
        assert_eq!(
            crate::render::incremental::take_cached_render_counts_for_test(),
            (1, 0, 0, 0, 0)
        );
        let next = target.derived_state.as_ref().unwrap();
        assert_eq!(
            next.render_blocks.materialize(),
            crate::render::incremental::render_blocks(&next.document, &target.schema)
        );
        assert_eq!(next.document_revision, target.revision());
        assert_eq!(next.schema_fingerprint, target.schema_fingerprint);
    }

    #[test]
    fn multi_operation_and_explicit_selection_inserts_use_sealed_eager_fallback() {
        use crate::yrs_engine::mutation::{
            reset_localized_lookup_counts_for_test, take_localized_lookup_counts_for_test,
        };

        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let mut transaction = insert_transaction(&engine, 70_141);
        transaction.operations.push(TypedOperation::InsertText {
            at: RevisionedPosition {
                offset: 2,
                kind: EditorOffsetKind::Scalar,
                affinity: Affinity::After,
            },
            text: "y".into(),
            marks: vec![],
        });
        reset_localized_lookup_counts_for_test();
        engine.apply_typed_transaction(transaction).unwrap();
        assert_eq!(take_localized_lookup_counts_for_test(), (1, 0, 0));

        let mut transaction = insert_transaction(&engine, 70_142);
        let point = RevisionedPosition {
            offset: 2,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::After,
        };
        transaction.selection_intent = SelectionIntent::Set(SelectionInput::Text {
            anchor: point,
            head: point,
        });
        reset_localized_lookup_counts_for_test();
        engine.apply_typed_transaction(transaction).unwrap();
        assert_eq!(take_localized_lookup_counts_for_test(), (1, 0, 0));
    }

    #[test]
    fn localized_insert_preserves_semantic_validation_error_precedence_over_lowering_limits() {
        fn constrained_engine() -> YrsDocumentEngine {
            let mut engine = transaction_engine();
            engine
                .import_json(
                    r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                    TransactionOrigin::DocumentImport,
                )
                .unwrap();
            engine.editing_limits.max_operations_per_transaction = 1;
            engine.resource_limits.max_document_depth = 1;
            engine.resource_limits.max_document_nodes = 1;
            engine
        }

        let localized = constrained_engine();
        let localized_error = localized
            .compile_typed_transaction(insert_transaction(&localized, 70_143))
            .unwrap_err();

        let eager = constrained_engine();
        let mut eager_transaction = insert_transaction(&eager, 70_143);
        eager_transaction.selection_intent = SelectionIntent::Set(SelectionInput::All);
        let eager_error = eager
            .compile_typed_transaction(eager_transaction)
            .unwrap_err();

        assert_eq!(localized_error, eager_error);
        assert_eq!(localized_error.code, "DOCUMENT_LIMIT_EXCEEDED");
    }

    #[test]
    fn engine_compile_reuses_all_cached_base_semantic_inputs() {
        use crate::yrs_engine::compiler::{
            reset_base_compilation_build_counts_for_test,
            take_base_compilation_build_counts_for_test,
        };

        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let mut transaction = insert_transaction(&engine, 70_002);
        let TypedOperation::InsertText { at, .. } = &mut transaction.operations[0] else {
            unreachable!()
        };
        at.offset = 2;
        let point = RevisionedPosition {
            offset: 2,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::Before,
        };
        transaction.selection_intent = SelectionIntent::Set(SelectionInput::Text {
            anchor: point,
            head: point,
        });
        reset_base_compilation_build_counts_for_test();

        engine.compile_typed_transaction(transaction).unwrap();

        assert_eq!(take_base_compilation_build_counts_for_test(), (0, 0, 0));
    }

    #[test]
    fn selection_only_revision_refreshes_the_cached_compilation_view() {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let point = |offset| RevisionedPosition {
            offset,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::Before,
        };
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 70_003,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: Vec::new(),
                selection_intent: SelectionIntent::Set(SelectionInput::Text {
                    anchor: point(1),
                    head: point(2),
                }),
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap();

        assert_eq!(
            engine.derived_state.as_ref().unwrap().legacy_selection,
            crate::selection::Selection::text(2, 3)
        );
        engine
            .compile_typed_transaction(TypedTransaction {
                request_id: 70_004,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: Vec::new(),
                selection_intent: SelectionIntent::Preserve,
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap();
    }

    #[test]
    fn changed_rich_command_derives_preview_map_and_render_at_most_once() {
        use crate::yrs_engine::derived_state::{
            reset_preview_derivation_counts_for_test, take_preview_derivation_counts_for_test,
        };

        let mut engine = transaction_engine();
        reset_preview_derivation_counts_for_test();

        engine
            .apply_command(70_007, TypedCommand::InsertText { text: "x".into() })
            .unwrap()
            .unwrap();

        let (position_maps, rendered_texts) = take_preview_derivation_counts_for_test();
        assert!(position_maps <= 1, "built {position_maps} preview maps");
        assert!(
            rendered_texts <= 1,
            "built {rendered_texts} preview renders"
        );
    }

    #[test]
    fn existing_text_command_skips_every_proved_document_wide_compiler_pass() {
        use crate::yrs_engine::observability::{
            reset_full_pass_counts_for_test, take_full_pass_counts_for_test, FullPassCounts,
        };

        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let caret = RevisionedPosition {
            offset: 1,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::After,
        };
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 70_008,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: Vec::new(),
                selection_intent: SelectionIntent::Set(SelectionInput::Text {
                    anchor: caret,
                    head: caret,
                }),
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap();
        hydrate_import_for_compile_test(&mut engine);
        reset_full_pass_counts_for_test();

        engine
            .apply_command(70_009, TypedCommand::InsertText { text: "x".into() })
            .unwrap()
            .unwrap();

        assert_eq!(
            take_full_pass_counts_for_test(),
            FullPassCounts {
                import_model_parses: 0,
                validated_evidence_constructions: 0,
                validation_certificate_constructions: 0,
                planner_simulations: 1,
                document_validations: 1,
                canonical_mark_tree_scans: 0,
                canonical_mark_validation_attempts: 0,
                canonical_mark_validation_completions: 0,
                canonical_mark_nodes_visited: 0,
                canonical_identity_predicate_nodes_visited: 3,
                canonical_projections: 1,
                canonical_serializations: 1,
                canonical_hashes: 1,
                affected_top_level_scans: 0,
                position_map_clones: 1,
                position_map_compactions: 1,
                rendered_text_derivations: 0,
                raw_document_text_scans: 1,
                document_node_count_scans: 0,
                render_limit_tree_scans: 0,
                render_identity_scans: 0,
                render_top_level_start_scans: 0,
                active_applicability_passes: 1,
                ordinary_step_applications: 1,
            }
        );
    }

    #[test]
    fn existing_text_admission_certificate_matches_legacy_compiler_and_commit() {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let point = RevisionedPosition {
            offset: 1,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::After,
        };
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 70_010,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: Vec::new(),
                selection_intent: SelectionIntent::Set(SelectionInput::Text {
                    anchor: point,
                    head: point,
                }),
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap();
        hydrate_import_for_compile_test(&mut engine);
        let transaction = TypedTransaction {
            request_id: 70_011,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertText {
                at: point,
                text: "🙂\\\"‍".into(),
                marks: Vec::new(),
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        };

        let compiled = engine
            .compile_typed_transaction(transaction.clone())
            .unwrap();
        let proof = compiled
            .localized_insert_admission
            .as_ref()
            .expect("strict-inside existing text produces E1 admission evidence")
            .clone();
        let read_txn = engine.doc.transact();
        let fragment = read_txn
            .get_xml_fragment(engine.fragment_name.as_str())
            .unwrap();
        let current = engine.derived_state.as_ref().unwrap();
        let admission_document_position = crate::yrs_engine::position::editor_offset_to_doc_pos(
            point.offset,
            point.kind,
            &current.rendered_text,
            &current.position_map,
            &current.document,
        )
        .unwrap();
        let validated = proof
            .validate_current(
                current,
                &transaction,
                admission_document_position,
                &read_txn,
                &fragment,
                &engine.resource_limits,
                &engine.editing_limits,
                engine.max_length,
                engine.yrs_state_epoch,
            )
            .expect("every private admission claim revalidates");
        let mut same_metrics_different_text = transaction.clone();
        let [TypedOperation::InsertText { text, .. }] =
            same_metrics_different_text.operations.as_mut_slice()
        else {
            unreachable!()
        };
        *text = "🙃\\\"‍".into();
        assert!(proof
            .validate_current(
                current,
                &same_metrics_different_text,
                admission_document_position,
                &read_txn,
                &fragment,
                &engine.resource_limits,
                &engine.editing_limits,
                engine.max_length,
                engine.yrs_state_epoch,
            )
            .is_none());
        for (claim, tampered) in proof.tampered_claims_for_test() {
            assert!(
                tampered
                    .validate_current(
                        current,
                        &transaction,
                        admission_document_position,
                        &read_txn,
                        &fragment,
                        &engine.resource_limits,
                        &engine.editing_limits,
                        engine.max_length,
                        engine.yrs_state_epoch,
                    )
                    .is_none(),
                "tampered private claim must fail closed: {claim}"
            );
        }
        drop(read_txn);
        let full_stats =
            DocumentValidator::validate(&compiled.preview, &engine.schema, &engine.resource_limits)
                .unwrap();
        assert_eq!(
            full_stats,
            engine
                .derived_state
                .as_ref()
                .unwrap()
                .validation_certificate
                .stats()
        );
        let artifact = compiled.canonical_artifact.as_ref().unwrap();
        assert_eq!(
            artifact.text_scalar_len(),
            validated.next_raw_text_scalars()
        );
        assert_eq!(
            artifact.text_utf8_bytes(),
            validated.next_raw_text_utf8_bytes()
        );
        assert_eq!(
            artifact.serialized_len(),
            validated.next_canonical_serialized_len()
        );
        assert_eq!(compiled.undo_units_bound, validated.history_undo_units());
        assert_eq!(
            compiled.replay_work_units_bound,
            validated.history_undo_units()
        );
        assert_eq!(
            compiled
                .preview_derivations
                .as_ref()
                .unwrap()
                .position_map
                .total_scalars(),
            validated.next_rendered_scalars()
        );
        let expected_fingerprint = artifact.sha256();
        let expected_operation_result = validated.operation_result().clone();
        let expected_stored_marks = validated.stored_marks().map(<[_]>::to_vec);
        let expected_rendered_scalars = validated.next_rendered_scalars();

        let result = engine
            .apply_compiled_transaction(compiled, true)
            .unwrap()
            .1
            .unwrap();
        assert_eq!(result.selection, expected_operation_result);
        assert_eq!(engine.stored_marks(), expected_stored_marks.as_deref());
        assert!(engine.can_undo());
        let next = engine.derived_state.as_ref().unwrap();
        assert_eq!(next.validation_certificate.stats(), full_stats);
        assert_eq!(
            next.validation_certificate.canonical_fingerprint(),
            expected_fingerprint
        );
        assert_eq!(next.position_map.total_scalars(), expected_rendered_scalars);
        assert_eq!(
            u32::try_from(next.rendered_text.chars().count()).unwrap(),
            expected_rendered_scalars
        );
    }

    #[test]
    fn admission_evidence_does_zero_work_before_envelope_admission() {
        use crate::yrs_engine::derived_state::{
            reset_localized_insert_admission_work_for_test,
            take_localized_insert_admission_work_for_test,
        };

        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let position = RevisionedPosition {
            offset: 1,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::After,
        };
        let insert = |base_document_revision, origin, text: &str| TypedTransaction {
            request_id: 70_012,
            base_document_revision,
            origin,
            operations: vec![TypedOperation::InsertText {
                at: position,
                text: text.into(),
                marks: Vec::new(),
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        };

        reset_localized_insert_admission_work_for_test();
        assert!(engine
            .compile_typed_transaction(insert(
                engine.revision().saturating_add(1),
                TransactionOrigin::LocalInput,
                "x",
            ))
            .is_err());
        assert_eq!(take_localized_insert_admission_work_for_test(), 0);

        reset_localized_insert_admission_work_for_test();
        assert!(engine
            .compile_typed_transaction(insert(
                engine.revision(),
                TransactionOrigin::RemoteSync,
                "x",
            ))
            .is_err());
        assert_eq!(take_localized_insert_admission_work_for_test(), 0);

        engine.editing_limits.max_operations_per_transaction = 1;
        let mut excess = insert(engine.revision(), TransactionOrigin::LocalInput, "x");
        excess.operations.push(excess.operations[0].clone());
        reset_localized_insert_admission_work_for_test();
        assert!(engine.compile_typed_transaction(excess).is_err());
        assert_eq!(take_localized_insert_admission_work_for_test(), 0);

        engine.resource_limits.max_input_bytes = 1;
        reset_localized_insert_admission_work_for_test();
        assert!(engine
            .compile_typed_transaction(insert(
                engine.revision(),
                TransactionOrigin::LocalInput,
                "oversized",
            ))
            .is_err());
        assert_eq!(take_localized_insert_admission_work_for_test(), 0);
    }

    #[test]
    fn localized_insert_admission_does_zero_work_before_cached_view_and_yrs_scan_admission() {
        use crate::yrs_engine::derived_state::{
            reset_localized_insert_admission_work_for_test,
            take_localized_insert_admission_work_for_test,
        };

        let fixture = || {
            let mut engine = transaction_engine();
            engine
                .import_json(
                    r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                    TransactionOrigin::DocumentImport,
                )
                .unwrap();
            hydrate_import_for_compile_test(&mut engine);
            engine
        };
        let transaction = |engine: &YrsDocumentEngine, request_id| TypedTransaction {
            request_id,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertText {
                at: RevisionedPosition {
                    offset: 1,
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::After,
                },
                text: "x".into(),
                marks: Vec::new(),
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        };

        let mut cached_view_rejection = fixture();
        let cached_transaction = transaction(&cached_view_rejection, 700_122);
        cached_view_rejection
            .derived_state
            .as_mut()
            .unwrap()
            .rendered_scalars += 1;
        reset_localized_insert_admission_work_for_test();
        assert!(cached_view_rejection
            .compile_typed_transaction(cached_transaction)
            .is_err());
        assert_eq!(take_localized_insert_admission_work_for_test(), 0);

        let mut yrs_scan_rejection = fixture();
        let scan_transaction = transaction(&yrs_scan_rejection, 700_123);
        yrs_scan_rejection.resource_limits.max_input_bytes = 8;
        reset_localized_insert_admission_work_for_test();
        let error = yrs_scan_rejection
            .compile_typed_transaction(scan_transaction)
            .unwrap_err();
        assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED");
        assert_eq!(error.details.as_ref().unwrap()["field"], "maxInputBytes");
        assert_eq!(take_localized_insert_admission_work_for_test(), 0);
    }

    #[test]
    fn localized_insert_admission_runs_before_mutation_preflight() {
        use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};
        use crate::yrs_engine::derived_state::{
            reset_localized_insert_admission_work_for_test,
            take_localized_insert_admission_work_for_test,
        };

        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        hydrate_import_for_compile_test(&mut engine);
        let transaction = TypedTransaction {
            request_id: 700_121,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertText {
                at: RevisionedPosition {
                    offset: 1,
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::After,
                },
                text: "x".into(),
                marks: Vec::new(),
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        };

        reset_localized_insert_admission_work_for_test();
        set_atomic_failpoint_for_test(Some(AtomicFailpoint::MutationPreflight));
        let result = engine.compile_typed_transaction(transaction);
        set_atomic_failpoint_for_test(None);

        assert!(result.is_err());
        assert_eq!(take_localized_insert_admission_work_for_test(), 1);
    }

    #[test]
    fn admission_evidence_rejects_unsupported_selection_and_history_contracts() {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let point = RevisionedPosition {
            offset: 1,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::After,
        };
        let transaction = |selection_intent, history_policy| TypedTransaction {
            request_id: 70_013,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertText {
                at: point,
                text: "x".into(),
                marks: Vec::new(),
            }],
            selection_intent,
            history_policy,
        };

        assert!(engine
            .compile_typed_transaction(transaction(SelectionIntent::Preserve, HistoryPolicy::Auto,))
            .unwrap()
            .localized_insert_admission
            .is_none());
        assert!(engine
            .compile_typed_transaction(transaction(
                SelectionIntent::UseOperationResult,
                HistoryPolicy::Skip,
            ))
            .unwrap()
            .localized_insert_admission
            .is_none());
    }

    #[test]
    fn localized_insert_admission_eligibility_is_exact() {
        let fixture = |marked: bool| {
            let mut engine = transaction_engine();
            let json = if marked {
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","marks":[{"type":"bold"}],"text":"abc"}]}]}"#
            } else {
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#
            };
            engine
                .import_json(json, TransactionOrigin::DocumentImport)
                .unwrap();
            hydrate_import_for_compile_test(&mut engine);
            engine
        };
        let point = |offset| RevisionedPosition {
            offset,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::After,
        };
        let transaction = |engine: &YrsDocumentEngine,
                           origin,
                           at,
                           text: &str,
                           marks,
                           selection_intent,
                           history_policy| TypedTransaction {
            request_id: 700_131,
            base_document_revision: engine.revision(),
            origin,
            operations: vec![TypedOperation::InsertText {
                at,
                text: text.into(),
                marks,
            }],
            selection_intent,
            history_policy,
        };

        let engine = fixture(false);
        for origin in [
            TransactionOrigin::LocalInput,
            TransactionOrigin::LocalCommand,
            TransactionOrigin::LocalApi,
        ] {
            assert!(engine
                .compile_typed_transaction(transaction(
                    &engine,
                    origin,
                    point(1),
                    "x",
                    Vec::new(),
                    SelectionIntent::UseOperationResult,
                    HistoryPolicy::Auto,
                ))
                .unwrap()
                .localized_insert_admission
                .is_some());
        }

        for boundary in [point(0), point(3)] {
            assert!(engine
                .compile_typed_transaction(transaction(
                    &engine,
                    TransactionOrigin::LocalInput,
                    boundary,
                    "x",
                    Vec::new(),
                    SelectionIntent::UseOperationResult,
                    HistoryPolicy::Auto,
                ))
                .unwrap()
                .localized_insert_admission
                .is_none());
        }

        for history_policy in [HistoryPolicy::Boundary, HistoryPolicy::Skip] {
            assert!(engine
                .compile_typed_transaction(transaction(
                    &engine,
                    TransactionOrigin::LocalInput,
                    point(1),
                    "x",
                    Vec::new(),
                    SelectionIntent::UseOperationResult,
                    history_policy,
                ))
                .unwrap()
                .localized_insert_admission
                .is_none());
        }
        for origin in [TransactionOrigin::LocalCommand, TransactionOrigin::LocalApi] {
            assert!(engine
                .compile_typed_transaction(transaction(
                    &engine,
                    origin,
                    point(1),
                    "x",
                    Vec::new(),
                    SelectionIntent::UseOperationResult,
                    HistoryPolicy::Boundary,
                ))
                .unwrap()
                .localized_insert_admission
                .is_none());
        }
        assert!(engine
            .compile_typed_transaction(transaction(
                &engine,
                TransactionOrigin::LocalInput,
                point(1),
                "x",
                Vec::new(),
                SelectionIntent::Preserve,
                HistoryPolicy::Auto,
            ))
            .unwrap()
            .localized_insert_admission
            .is_none());
        assert!(engine
            .compile_typed_transaction(transaction(
                &engine,
                TransactionOrigin::LocalInput,
                point(1),
                "x",
                Vec::new(),
                SelectionIntent::Set(SelectionInput::Text {
                    anchor: point(1),
                    head: point(1),
                }),
                HistoryPolicy::Auto,
            ))
            .unwrap()
            .localized_insert_admission
            .is_none());

        let mut multiple = transaction(
            &engine,
            TransactionOrigin::LocalInput,
            point(1),
            "x",
            Vec::new(),
            SelectionIntent::UseOperationResult,
            HistoryPolicy::Auto,
        );
        multiple.operations.push(multiple.operations[0].clone());
        assert!(engine
            .compile_typed_transaction(multiple)
            .unwrap()
            .localized_insert_admission
            .is_none());

        let marked_engine = fixture(true);
        let bold = vec![Mark::new("bold".into(), HashMap::new())];
        assert!(marked_engine
            .compile_typed_transaction(transaction(
                &marked_engine,
                TransactionOrigin::LocalInput,
                point(1),
                "x",
                bold,
                SelectionIntent::UseOperationResult,
                HistoryPolicy::Auto,
            ))
            .unwrap()
            .localized_insert_admission
            .is_some());
        assert!(marked_engine
            .compile_typed_transaction(transaction(
                &marked_engine,
                TransactionOrigin::LocalInput,
                point(1),
                "x",
                Vec::new(),
                SelectionIntent::UseOperationResult,
                HistoryPolicy::Auto,
            ))
            .unwrap()
            .localized_insert_admission
            .is_none());
    }

    #[test]
    fn localized_insert_admission_preserves_generic_results_errors_and_full_pass_counts() {
        use crate::yrs_engine::observability::{
            reset_full_pass_counts_for_test, take_full_pass_counts_for_test,
        };

        let fixture = || {
            let mut engine = transaction_engine();
            engine
                .import_json(
                    r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                    TransactionOrigin::DocumentImport,
                )
                .unwrap();
            hydrate_import_for_compile_test(&mut engine);
            engine
        };
        let transaction = |engine: &YrsDocumentEngine, request_id, marks| TypedTransaction {
            request_id,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertText {
                at: RevisionedPosition {
                    offset: 1,
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::After,
                },
                text: "x".into(),
                marks,
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        };

        let mut admitted = fixture();
        reset_full_pass_counts_for_test();
        let admitted_result = admitted
            .apply_typed_transaction_with_result(transaction(&admitted, 700_132, Vec::new()))
            .unwrap();
        let admitted_counts = take_full_pass_counts_for_test();

        let mut generic = fixture();
        generic.derived_state.as_mut().unwrap().localized_text_index = None;
        reset_full_pass_counts_for_test();
        let generic_result = generic
            .apply_typed_transaction_with_result(transaction(&generic, 700_132, Vec::new()))
            .unwrap();
        let generic_counts = take_full_pass_counts_for_test();

        assert_eq!(admitted_result, generic_result);
        assert_eq!(admitted.document_json(), generic.document_json());
        assert_eq!(admitted_counts.ordinary_step_applications, 0);
        assert_eq!(generic_counts.ordinary_step_applications, 1);
        assert_eq!(admitted.can_undo(), generic.can_undo());
        assert_eq!(admitted.can_redo(), generic.can_redo());

        let admitted_undo = admitted.undo(700_141).unwrap();
        let generic_undo = generic.undo(700_141).unwrap();
        assert_eq!(admitted_undo, generic_undo);
        assert_eq!(admitted.document_json(), generic.document_json());
        assert_eq!(admitted.can_undo(), generic.can_undo());
        assert_eq!(admitted.can_redo(), generic.can_redo());

        let admitted_redo = admitted.redo(700_142).unwrap();
        let generic_redo = generic.redo(700_142).unwrap();
        assert_eq!(admitted_redo, generic_redo);
        assert_eq!(admitted.document_json(), generic.document_json());
        assert_eq!(admitted.can_undo(), generic.can_undo());
        assert_eq!(admitted.can_redo(), generic.can_redo());

        let invalid_mark = vec![Mark::new("unknown".into(), HashMap::new())];
        let mut admitted_error_engine = fixture();
        let mut generic_error_engine = fixture();
        generic_error_engine
            .derived_state
            .as_mut()
            .unwrap()
            .localized_text_index = None;
        let admitted_error = admitted_error_engine
            .apply_typed_transaction_with_result(transaction(
                &admitted_error_engine,
                700_133,
                invalid_mark.clone(),
            ))
            .unwrap_err();
        let generic_error = generic_error_engine
            .apply_typed_transaction_with_result(transaction(
                &generic_error_engine,
                700_133,
                invalid_mark,
            ))
            .unwrap_err();
        assert_eq!(admitted_error, generic_error);
        assert_eq!(
            admitted_error_engine.document_json(),
            generic_error_engine.document_json()
        );
    }

    #[test]
    fn localized_insert_compile_only_skips_every_proved_full_pass() {
        use crate::model::node::{
            reset_deep_node_payload_clones_for_test, take_deep_node_payload_clones_for_test,
        };
        use crate::yrs_engine::observability::{
            reset_full_pass_counts_for_test, take_full_pass_counts_for_test, FullPassCounts,
        };

        let fixture = || {
            let mut engine = transaction_engine();
            engine
                .import_json(
                    r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                    TransactionOrigin::DocumentImport,
                )
                .unwrap();
            hydrate_import_for_compile_test(&mut engine);
            engine
        };
        let transaction = |engine: &YrsDocumentEngine, request_id| TypedTransaction {
            request_id,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertText {
                at: RevisionedPosition {
                    offset: 1,
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::After,
                },
                text: "x".into(),
                marks: Vec::new(),
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        };

        let eligible = fixture();
        reset_full_pass_counts_for_test();
        let compiled = eligible
            .compile_typed_transaction(transaction(&eligible, 700_134))
            .unwrap();
        assert_eq!(compiled.affected_top_level_blocks, vec![0]);
        assert_eq!(
            take_full_pass_counts_for_test(),
            FullPassCounts {
                canonical_projections: 1,
                canonical_serializations: 2,
                canonical_hashes: 1,
                position_map_clones: 1,
                position_map_compactions: 1,
                render_identity_scans: 0,
                ..FullPassCounts::default()
            }
        );

        let mut generic = fixture();
        generic.derived_state.as_mut().unwrap().localized_text_index = None;
        reset_full_pass_counts_for_test();
        generic
            .compile_typed_transaction(transaction(&generic, 700_135))
            .unwrap();
        assert_eq!(
            take_full_pass_counts_for_test(),
            FullPassCounts {
                document_validations: 2,
                canonical_mark_tree_scans: 1,
                canonical_mark_validation_attempts: 1,
                canonical_mark_validation_completions: 1,
                canonical_mark_nodes_visited: 3,
                canonical_identity_predicate_nodes_visited: 3,
                canonical_projections: 1,
                canonical_serializations: 1,
                canonical_hashes: 0,
                affected_top_level_scans: 1,
                position_map_clones: 1,
                position_map_compactions: 1,
                rendered_text_derivations: 1,
                raw_document_text_scans: 2,
                document_node_count_scans: 1,
                render_identity_scans: 0,
                ordinary_step_applications: 1,
                ..FullPassCounts::default()
            }
        );

        let mut wide = transaction_engine();
        let content = (0..160)
            .map(|index| {
                json!({
                    "type": "paragraph",
                    "content": [{
                        "type": "text",
                        "text": format!("{index:04} {}", "x".repeat(214))
                    }]
                })
            })
            .collect::<Vec<_>>();
        wide.import_json(
            &json!({"type": "doc", "content": content}).to_string(),
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
        let rendered = &wide.derived_state.as_ref().unwrap().rendered_text;
        let needle = "0159 ";
        let needle_byte = rendered.find(needle).unwrap();
        let offset = u32::try_from(rendered[..needle_byte].chars().count() + needle.len()).unwrap();
        reset_deep_node_payload_clones_for_test();
        wide.compile_typed_transaction(TypedTransaction {
            request_id: 700_143,
            base_document_revision: wide.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertText {
                at: RevisionedPosition {
                    offset,
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::After,
                },
                text: "y".into(),
                marks: Vec::new(),
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        })
        .unwrap();
        assert_eq!(
            take_deep_node_payload_clones_for_test(),
            0,
            "localized reconstruction must copy only immutable node handles"
        );
    }

    #[test]
    fn localized_insert_semantic_preview_matches_forced_generic_matrix() {
        fn assert_compiled_parity(
            localized: &crate::yrs_engine::compiler::CompiledTransaction,
            generic: &crate::yrs_engine::compiler::CompiledTransaction,
        ) {
            assert_eq!(localized.preview, generic.preview);
            let localized_artifact = localized.canonical_artifact.as_ref().unwrap();
            let generic_artifact = generic.canonical_artifact.as_ref().unwrap();
            assert_eq!(localized_artifact.value(), generic_artifact.value());
            assert_eq!(localized_artifact.sha256(), generic_artifact.sha256());
            assert_eq!(
                localized_artifact.serialized_len(),
                generic_artifact.serialized_len()
            );
            assert_eq!(
                localized_artifact.text_scalar_len(),
                generic_artifact.text_scalar_len()
            );
            assert_eq!(
                localized_artifact.text_utf8_bytes(),
                generic_artifact.text_utf8_bytes()
            );
            assert!(localized_artifact.matches_document(&localized.preview));
            assert!(generic_artifact.matches_document(&generic.preview));
            assert_eq!(
                localized.composed_map.ranges(),
                generic.composed_map.ranges()
            );
            assert_eq!(localized.selection_plan, generic.selection_plan);
            assert_eq!(
                localized.relative_selection_plan,
                generic.relative_selection_plan
            );
            assert_eq!(localized.stored_marks_plan, generic.stored_marks_plan);
            assert_eq!(localized.history_class, generic.history_class);
            assert_eq!(localized.undo_units_bound, generic.undo_units_bound);
            assert_eq!(
                localized.replay_work_units_bound,
                generic.replay_work_units_bound
            );
            assert_eq!(localized.encoded_growth_bound, generic.encoded_growth_bound);
            assert_eq!(localized.authored_clock_units, generic.authored_clock_units);
            assert_eq!(
                localized.affected_top_level_blocks,
                generic.affected_top_level_blocks
            );
            assert_eq!(localized.position_update_mode, generic.position_update_mode);
            assert_eq!(
                format!("{:?}", localized.mutation_plan.actions),
                format!("{:?}", generic.mutation_plan.actions)
            );
            assert_eq!(
                localized.mutation_plan.compilation_work_for_test(),
                generic.mutation_plan.compilation_work_for_test()
            );
            assert_eq!(
                localized.mutation_plan.expected_preflight_work_for_test(),
                generic.mutation_plan.expected_preflight_work_for_test()
            );
            assert_eq!(
                localized.mutation_plan.scan_work,
                generic.mutation_plan.scan_work
            );
            let localized_derived = localized.preview_derivations.as_ref().unwrap();
            let generic_derived = generic.preview_derivations.as_ref().unwrap();
            assert_eq!(
                localized_derived.rendered_text,
                generic_derived.rendered_text
            );
            assert_eq!(
                localized_derived.rendered_scalars,
                generic_derived.rendered_scalars
            );
            assert_eq!(
                localized_derived.document_text_bytes,
                generic_derived.document_text_bytes
            );
            assert_eq!(
                localized_derived.document_node_count,
                generic_derived.document_node_count
            );
            assert_eq!(
                format!("{:?}", localized_derived.position_map),
                format!("{:?}", generic_derived.position_map)
            );
        }

        let cases = [
            (
                "ascii",
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                "abc",
                1usize,
                "x",
                Vec::new(),
                vec![0],
            ),
            (
                "non-bmp-escaped-control",
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                "abc",
                1,
                "🙂\\\"\n\u{1}",
                Vec::new(),
                vec![0],
            ),
            (
                "canonical-mark",
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","marks":[{"type":"bold"}],"text":"abc"}]}]}"#,
                "abc",
                1,
                "x",
                vec![Mark::new("bold".into(), HashMap::new())],
                vec![0],
            ),
            (
                "fragmented-mark-leaves",
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","marks":[{"type":"bold"}],"text":"ab"},{"type":"text","marks":[{"type":"italic"}],"text":"cd"}]}]}"#,
                "cd",
                1,
                "🙂",
                vec![Mark::new("italic".into(), HashMap::new())],
                vec![0],
            ),
            (
                "deep-nesting",
                r#"{"type":"doc","content":[{"type":"blockquote","content":[{"type":"blockquote","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}]}]}"#,
                "abc",
                1,
                "x",
                Vec::new(),
                vec![0],
            ),
            (
                "list-prefix",
                r#"{"type":"doc","content":[{"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}]}]}"#,
                "abc",
                1,
                "x",
                Vec::new(),
                vec![0],
            ),
            (
                "third-top-level",
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"first"}]},{"type":"paragraph","content":[{"type":"text","text":"second"}]},{"type":"paragraph","content":[{"type":"text","text":"third"}]}]}"#,
                "third",
                1,
                "x",
                Vec::new(),
                vec![1, 2],
            ),
        ];

        for (case, json, needle, inside, inserted, marks, expected_affected) in cases {
            let mut engine = transaction_engine();
            engine
                .import_json(json, TransactionOrigin::DocumentImport)
                .unwrap();
            hydrate_import_for_compile_test(&mut engine);
            let rendered = &engine.derived_state.as_ref().unwrap().rendered_text;
            let needle_byte = rendered.find(needle).unwrap();
            let offset = u32::try_from(rendered[..needle_byte].chars().count() + inside).unwrap();
            let transaction = TypedTransaction {
                request_id: 700_136,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: RevisionedPosition {
                        offset,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    text: inserted.into(),
                    marks,
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            };

            let localized = engine
                .compile_typed_transaction(transaction.clone())
                .unwrap();
            assert!(localized.localized_insert_admission.is_some(), "{case}");
            assert_eq!(
                localized.affected_top_level_blocks, expected_affected,
                "{case}"
            );
            let saved_index = engine
                .derived_state
                .as_mut()
                .unwrap()
                .localized_text_index
                .take();
            let generic = engine
                .compile_typed_transaction(transaction.clone())
                .unwrap();
            engine.derived_state.as_mut().unwrap().localized_text_index = saved_index;
            assert_compiled_parity(&localized, &generic);

            let localized_result = engine
                .apply_compiled_transaction(localized, true)
                .unwrap()
                .1
                .unwrap();
            let mut generic_engine = transaction_engine();
            generic_engine
                .import_json(json, TransactionOrigin::DocumentImport)
                .unwrap();
            generic_engine
                .derived_state
                .as_mut()
                .unwrap()
                .localized_text_index = None;
            let generic_compiled = generic_engine
                .compile_typed_transaction(transaction)
                .unwrap();
            let generic_result = generic_engine
                .apply_compiled_transaction(generic_compiled, true)
                .unwrap()
                .1
                .unwrap();
            assert_eq!(localized_result, generic_result, "{case}");
            assert_eq!(
                engine.document_json(),
                generic_engine.document_json(),
                "{case}"
            );
            let localized_state = engine.derived_state.as_ref().unwrap();
            let generic_state = generic_engine.derived_state.as_ref().unwrap();
            assert_eq!(
                localized_state.validation_certificate, generic_state.validation_certificate,
                "{case}"
            );
            assert_eq!(
                localized_state.localized_text_index, generic_state.localized_text_index,
                "{case}"
            );
            assert_eq!(
                localized_state.canonical_artifact.value(),
                generic_state.canonical_artifact.value(),
                "{case}"
            );
            assert_eq!(
                localized_state.canonical_artifact.sha256(),
                generic_state.canonical_artifact.sha256(),
                "{case}"
            );
            assert_eq!(
                localized_state.rendered_text, generic_state.rendered_text,
                "{case}"
            );
            assert_eq!(engine.can_undo(), generic_engine.can_undo(), "{case}");
            assert_eq!(engine.can_redo(), generic_engine.can_redo(), "{case}");
            assert_eq!(
                engine.undo(700_151).unwrap(),
                generic_engine.undo(700_151).unwrap(),
                "{case}"
            );
            assert_eq!(
                engine.document_json(),
                generic_engine.document_json(),
                "{case}"
            );
            assert_eq!(
                engine.redo(700_152).unwrap(),
                generic_engine.redo(700_152).unwrap(),
                "{case}"
            );
            assert_eq!(
                engine.document_json(),
                generic_engine.document_json(),
                "{case}"
            );
        }

        use crate::yrs_engine::compiler::force_localized_semantic_allocation_failure_for_test;
        use crate::yrs_engine::observability::{
            reset_full_pass_counts_for_test, take_full_pass_counts_for_test,
        };
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        hydrate_import_for_compile_test(&mut engine);
        let transaction = TypedTransaction {
            request_id: 700_139,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertText {
                at: RevisionedPosition {
                    offset: 1,
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::After,
                },
                text: "x".into(),
                marks: Vec::new(),
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        };
        reset_full_pass_counts_for_test();
        force_localized_semantic_allocation_failure_for_test(true);
        let fallback = engine.compile_typed_transaction(transaction.clone());
        force_localized_semantic_allocation_failure_for_test(false);
        let fallback = fallback.unwrap();
        assert!(fallback.localized_insert_admission.is_some());
        assert_eq!(
            take_full_pass_counts_for_test().ordinary_step_applications,
            1
        );
        let saved_index = engine
            .derived_state
            .as_mut()
            .unwrap()
            .localized_text_index
            .take();
        let generic = engine.compile_typed_transaction(transaction).unwrap();
        engine.derived_state.as_mut().unwrap().localized_text_index = saved_index;
        assert_compiled_parity(&fallback, &generic);
    }

    #[test]
    fn localized_insert_exact_limits_and_one_under_errors_match_generic() {
        use crate::yrs_engine::observability::{
            reset_full_pass_counts_for_test, take_full_pass_counts_for_test,
        };
        use crate::yrs_engine::EditingLimits;

        fn fixture(max_length: Option<u32>, editing_limits: EditingLimits) -> YrsDocumentEngine {
            let mut engine = YrsDocumentEngine::new(YrsEngineConfig {
                schema: tiptap_schema(),
                fragment_name: "prosemirror".into(),
                initialization_mode: crate::yrs_engine::InitializationMode::LocalEmpty,
                resource_limits: ResourceLimits::default(),
                editing_limits,
                max_length,
                scope: Some(crate::yrs_engine::DocumentScope {
                    document_id: "doc".into(),
                    lineage_id: "lineage".into(),
                }),
            })
            .unwrap();
            engine
                .import_json(
                    r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                    TransactionOrigin::DocumentImport,
                )
                .unwrap();
            hydrate_import_for_compile_test(&mut engine);
            engine
        }

        fn transaction(engine: &YrsDocumentEngine) -> TypedTransaction {
            TypedTransaction {
                request_id: 700_140,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: RevisionedPosition {
                        offset: 1,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    text: "xy".into(),
                    marks: Vec::new(),
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            }
        }

        fn assert_error_pair(
            localized: YrsDocumentEngine,
            mut generic: YrsDocumentEngine,
            field: &str,
        ) {
            generic.derived_state.as_mut().unwrap().localized_text_index = None;
            reset_full_pass_counts_for_test();
            let localized_error = localized
                .compile_typed_transaction(transaction(&localized))
                .unwrap_err();
            assert_eq!(
                take_full_pass_counts_for_test().ordinary_step_applications,
                1,
                "{field} must silently fall back to generic compilation"
            );
            let generic_error = generic
                .compile_typed_transaction(transaction(&generic))
                .unwrap_err();
            assert_eq!(localized_error, generic_error);
            assert_eq!(localized_error.details.as_ref().unwrap()["field"], field);
        }

        let probe_engine = fixture(None, EditingLimits::default());
        let probe = probe_engine
            .compile_typed_transaction(transaction(&probe_engine))
            .unwrap();
        let exact_output = probe.canonical_artifact.as_ref().unwrap().serialized_len();
        let exact_undo = probe.undo_units_bound;

        let exact_length = fixture(Some(5), EditingLimits::default());
        assert!(exact_length
            .compile_typed_transaction(transaction(&exact_length))
            .unwrap()
            .localized_insert_admission
            .is_some());
        let rejected_length = fixture(Some(4), EditingLimits::default());
        let generic_length = fixture(Some(4), EditingLimits::default());
        assert_error_pair(rejected_length, generic_length, "maxLength");

        let exact_output_limits = EditingLimits {
            max_derived_output_bytes: exact_output,
            ..EditingLimits::default()
        };
        let exact_output_engine = fixture(None, exact_output_limits);
        assert!(exact_output_engine
            .compile_typed_transaction(transaction(&exact_output_engine))
            .unwrap()
            .localized_insert_admission
            .is_some());
        let rejected_output_limits = EditingLimits {
            max_derived_output_bytes: exact_output - 1,
            ..EditingLimits::default()
        };
        let rejected_output = fixture(None, rejected_output_limits.clone());
        let generic_output = fixture(None, rejected_output_limits);
        assert_error_pair(rejected_output, generic_output, "maxDerivedOutputBytes");

        let exact_undo_limits = EditingLimits {
            max_undo_retained_units: exact_undo,
            ..EditingLimits::default()
        };
        let exact_undo_engine = fixture(None, exact_undo_limits);
        assert!(exact_undo_engine
            .compile_typed_transaction(transaction(&exact_undo_engine))
            .unwrap()
            .localized_insert_admission
            .is_some());
        let rejected_undo_limits = EditingLimits {
            max_undo_retained_units: exact_undo - 1,
            ..EditingLimits::default()
        };
        let rejected_undo = fixture(None, rejected_undo_limits.clone());
        let generic_undo = fixture(None, rejected_undo_limits);
        assert_error_pair(rejected_undo, generic_undo, "maxUndoRetainedUnits");
    }

    #[test]
    fn localized_index_promotion_allocation_failures_drop_only_optional_index() {
        use crate::yrs_engine::derived_state::{
            force_localized_index_allocation_stage_for_test, force_localized_index_budget_for_test,
            reset_localized_index_lifecycle_counts_for_test,
            take_localized_index_lifecycle_counts_for_test, LocalizedIndexAllocationStage,
        };

        fn fixture() -> YrsDocumentEngine {
            let mut engine = transaction_engine();
            engine
                .import_json(
                    r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                    TransactionOrigin::DocumentImport,
                )
                .unwrap();
            hydrate_import_for_compile_test(&mut engine);
            engine
        }
        fn transaction(engine: &YrsDocumentEngine, request_id: u64) -> TypedTransaction {
            TypedTransaction {
                request_id,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: RevisionedPosition {
                        offset: 1,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    text: "x".into(),
                    marks: Vec::new(),
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            }
        }

        let mut baseline = fixture();
        let baseline_result = baseline
            .apply_typed_transaction_with_result(transaction(&baseline, 700_144))
            .unwrap();
        let baseline_json = baseline.document_json();

        for (index, stage) in [
            LocalizedIndexAllocationStage::PromotionClone,
            LocalizedIndexAllocationStage::PromotionGrowth,
            LocalizedIndexAllocationStage::PromotionUpdate,
        ]
        .into_iter()
        .enumerate()
        {
            let mut engine = fixture();
            reset_localized_index_lifecycle_counts_for_test();
            force_localized_index_allocation_stage_for_test(Some(stage));
            let compiled = engine.compile_typed_transaction(transaction(
                &engine,
                700_145 + u64::try_from(index).unwrap(),
            ));
            force_localized_index_allocation_stage_for_test(None);
            let result = engine
                .apply_compiled_transaction(compiled.unwrap(), true)
                .unwrap()
                .1
                .unwrap();
            assert_eq!(result.changed, baseline_result.changed, "{stage:?}");
            assert_eq!(result.selection, baseline_result.selection, "{stage:?}");
            assert_eq!(engine.document_json(), baseline_json, "{stage:?}");
            assert!(engine.can_undo(), "{stage:?}");
            assert!(engine
                .derived_state
                .as_ref()
                .unwrap()
                .localized_text_index
                .is_none());
            assert_eq!(
                take_localized_index_lifecycle_counts_for_test(),
                (0, 1, 0, 1),
                "{stage:?}"
            );
        }

        let mut engine = fixture();
        reset_localized_index_lifecycle_counts_for_test();
        force_localized_index_budget_for_test(Some(0));
        let compiled = engine.compile_typed_transaction(transaction(&engine, 700_149));
        force_localized_index_budget_for_test(None);
        engine
            .apply_compiled_transaction(compiled.unwrap(), true)
            .unwrap();
        assert_eq!(engine.document_json(), baseline_json);
        assert!(engine
            .derived_state
            .as_ref()
            .unwrap()
            .localized_text_index
            .is_none());
        assert_eq!(
            take_localized_index_lifecycle_counts_for_test(),
            (0, 1, 0, 1)
        );
    }

    #[test]
    fn localized_index_promotion_obeys_exact_transient_budget_boundary() {
        use crate::yrs_engine::derived_state::{
            force_localized_index_budget_for_test, reset_localized_index_lifecycle_counts_for_test,
            take_localized_index_lifecycle_counts_for_test,
        };

        fn fixture() -> YrsDocumentEngine {
            let mut engine = transaction_engine();
            engine
                .import_json(
                    r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                    TransactionOrigin::DocumentImport,
                )
                .unwrap();
            hydrate_import_for_compile_test(&mut engine);
            engine
        }

        fn transaction(engine: &YrsDocumentEngine, request_id: u64) -> TypedTransaction {
            TypedTransaction {
                request_id,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: RevisionedPosition {
                        offset: 1,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    text: "x".into(),
                    marks: Vec::new(),
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            }
        }

        fn history_audit(engine: &YrsDocumentEngine) -> (bool, bool, u64, (usize, usize, bool)) {
            (
                engine.can_undo(),
                engine.can_redo(),
                engine.history.retained_units(0).unwrap(),
                engine.history.replay_audit_for_test(),
            )
        }

        let mut exact = fixture();
        let exact_budget = exact
            .derived_state
            .as_ref()
            .unwrap()
            .localized_text_index
            .as_ref()
            .unwrap()
            .promotion_transient_budget_for_test()
            .unwrap();
        reset_localized_index_lifecycle_counts_for_test();
        force_localized_index_budget_for_test(Some(exact_budget));
        let exact_compiled = exact
            .compile_typed_transaction(transaction(&exact, 700_162))
            .unwrap();
        force_localized_index_budget_for_test(None);
        let exact_result = exact
            .apply_compiled_transaction(exact_compiled, true)
            .unwrap()
            .1
            .unwrap();
        assert_eq!(
            take_localized_index_lifecycle_counts_for_test(),
            (0, 1, 1, 0)
        );

        let mut generic = fixture();
        generic.derived_state.as_mut().unwrap().localized_text_index = None;
        let generic_transaction = transaction(&generic, 700_162);
        let generic_result = generic
            .apply_typed_transaction_with_result(generic_transaction)
            .unwrap();
        assert_eq!(exact_result, generic_result);
        assert_eq!(exact.document_json(), generic.document_json());
        assert_eq!(history_audit(&exact), history_audit(&generic));
        assert_eq!(
            exact.derived_state.as_ref().unwrap().localized_text_index,
            generic.derived_state.as_ref().unwrap().localized_text_index
        );

        let mut one_under = fixture();
        reset_localized_index_lifecycle_counts_for_test();
        force_localized_index_budget_for_test(Some(exact_budget - 1));
        let one_under_compiled = one_under
            .compile_typed_transaction(transaction(&one_under, 700_162))
            .unwrap();
        force_localized_index_budget_for_test(None);
        let one_under_result = one_under
            .apply_compiled_transaction(one_under_compiled, true)
            .unwrap()
            .1
            .unwrap();
        assert_eq!(one_under_result, generic_result);
        assert_eq!(one_under.document_json(), generic.document_json());
        assert_eq!(history_audit(&one_under), history_audit(&generic));
        assert!(one_under
            .derived_state
            .as_ref()
            .unwrap()
            .localized_text_index
            .is_none());
        assert_eq!(
            take_localized_index_lifecycle_counts_for_test(),
            (0, 1, 0, 1)
        );
    }

    #[test]
    fn every_localized_derived_evidence_tamper_falls_back_before_write() {
        use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};
        use crate::yrs_engine::derived_state::{
            reset_localized_index_lifecycle_counts_for_test,
            take_localized_index_lifecycle_counts_for_test, PreparedDerivedEvidence,
        };

        for case in PreparedDerivedEvidence::tamper_cases_for_test() {
            let mut engine = transaction_engine();
            engine
                .import_json(
                    r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                    TransactionOrigin::DocumentImport,
                )
                .unwrap();
            hydrate_import_for_compile_test(&mut engine);
            let mut compiled = engine
                .compile_typed_transaction(TypedTransaction {
                    request_id: 700_150,
                    base_document_revision: engine.revision(),
                    origin: TransactionOrigin::LocalInput,
                    operations: vec![TypedOperation::InsertText {
                        at: RevisionedPosition {
                            offset: 1,
                            kind: EditorOffsetKind::Scalar,
                            affinity: Affinity::After,
                        },
                        text: "x".into(),
                        marks: Vec::new(),
                    }],
                    selection_intent: SelectionIntent::UseOperationResult,
                    history_policy: HistoryPolicy::Auto,
                })
                .unwrap();
            compiled
                .prepared_derived_evidence
                .as_mut()
                .unwrap()
                .tamper_for_test(case);
            let before = atomic_audit(&engine);
            reset_localized_index_lifecycle_counts_for_test();
            set_atomic_failpoint_for_test(Some(AtomicFailpoint::DurableMetadataAdmission));
            let applied = engine.apply_compiled_transaction(compiled, true);
            set_atomic_failpoint_for_test(None);
            assert!(applied.is_err(), "{case}");
            assert_eq!(atomic_audit(&engine), before, "{case}");
            assert_eq!(
                take_localized_index_lifecycle_counts_for_test(),
                (1, 0, 0, 0),
                "{case} must prepare generic evidence before the failpoint"
            );
        }
    }

    #[test]
    fn every_localized_render_proof_tamper_falls_back_before_write() {
        use crate::render::incremental::{
            reset_cached_render_counts_for_test, reset_localized_render_transition_counts_for_test,
            take_cached_render_counts_for_test, take_localized_render_transition_counts_for_test,
        };
        use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};
        use crate::yrs_engine::derived_state::PreparedDerivedEvidence;
        use crate::yrs_engine::observability::{
            reset_full_pass_counts_for_test, take_full_pass_counts_for_test,
        };

        let cases = PreparedDerivedEvidence::localized_render_tamper_cases_for_test()
            .iter()
            .copied()
            .chain(std::iter::once("affectedRange"));
        for case in cases {
            let mut engine = transaction_engine();
            engine
                .import_json(
                    r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                    TransactionOrigin::DocumentImport,
                )
                .unwrap();
            hydrate_import_for_compile_test(&mut engine);
            let mut compiled = engine
                .compile_typed_transaction(TypedTransaction {
                    request_id: 700_151,
                    base_document_revision: engine.revision(),
                    origin: TransactionOrigin::LocalInput,
                    operations: vec![TypedOperation::InsertText {
                        at: RevisionedPosition {
                            offset: 1,
                            kind: EditorOffsetKind::Scalar,
                            affinity: Affinity::After,
                        },
                        text: "x".into(),
                        marks: Vec::new(),
                    }],
                    selection_intent: SelectionIntent::UseOperationResult,
                    history_policy: HistoryPolicy::Auto,
                })
                .unwrap();
            if case == "affectedRange" {
                compiled.affected_top_level_blocks.clear();
            } else {
                compiled
                    .prepared_derived_evidence
                    .as_mut()
                    .unwrap()
                    .tamper_localized_render_for_test(case);
            }
            let before = atomic_audit(&engine);
            reset_full_pass_counts_for_test();
            reset_cached_render_counts_for_test();
            reset_localized_render_transition_counts_for_test();
            set_atomic_failpoint_for_test(Some(AtomicFailpoint::DurableMetadataAdmission));
            let error = engine
                .apply_compiled_transaction(compiled, true)
                .expect_err("durable metadata failpoint must abort the fallback commit");
            set_atomic_failpoint_for_test(None);

            assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{case}");
            assert_eq!(atomic_audit(&engine), before, "{case}");
            let passes = take_full_pass_counts_for_test();
            assert_eq!(passes.render_limit_tree_scans, 1, "{case}");
            assert_eq!(passes.render_identity_scans, 0, "{case}");
            assert_eq!(passes.render_top_level_start_scans, 1, "{case}");
            assert_eq!(
                take_cached_render_counts_for_test(),
                (0, 1, 1, 0, 0),
                "{case}"
            );
            assert_eq!(
                take_localized_render_transition_counts_for_test(),
                (1, 0, 1),
                "{case}"
            );
        }
    }

    #[test]
    fn malformed_multiblock_localized_render_ranges_fall_back_exactly() {
        use crate::render::incremental::{
            reset_cached_render_counts_for_test, reset_localized_render_transition_counts_for_test,
            take_cached_render_counts_for_test, take_localized_render_transition_counts_for_test,
        };
        use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};
        use crate::yrs_engine::observability::{
            reset_full_pass_counts_for_test, take_full_pass_counts_for_test, FullPassCounts,
        };

        struct RangeAudit {
            error: crate::yrs_engine::OperationError,
            cached_counts: (usize, usize, usize, usize, usize),
            lifecycle_counts: (usize, usize, usize),
            full_pass_counts: FullPassCounts,
        }

        fn run(affected: Option<Vec<usize>>) -> RangeAudit {
            let mut engine = transaction_engine();
            engine
                .import_json(
                    r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"aaa"}]},{"type":"paragraph","content":[{"type":"text","text":"bbb"}]},{"type":"paragraph","content":[{"type":"text","text":"ccc"}]},{"type":"paragraph","content":[{"type":"text","text":"ddd"}]}]}"#,
                    TransactionOrigin::DocumentImport,
                )
                .unwrap();
            hydrate_import_for_compile_test(&mut engine);
            let mut compiled = engine
                .compile_typed_transaction(TypedTransaction {
                    request_id: 700_154,
                    base_document_revision: engine.revision(),
                    origin: TransactionOrigin::LocalInput,
                    operations: vec![TypedOperation::InsertText {
                        at: RevisionedPosition {
                            offset: 9,
                            kind: EditorOffsetKind::Scalar,
                            affinity: Affinity::After,
                        },
                        text: "x".into(),
                        marks: Vec::new(),
                    }],
                    selection_intent: SelectionIntent::UseOperationResult,
                    history_policy: HistoryPolicy::Auto,
                })
                .unwrap();
            assert_eq!(compiled.affected_top_level_blocks, [1, 2, 3]);
            match affected {
                Some(affected) => compiled.affected_top_level_blocks = affected,
                None => compiled.localized_semantic_used = false,
            }
            let before = atomic_audit(&engine);
            reset_full_pass_counts_for_test();
            reset_cached_render_counts_for_test();
            reset_localized_render_transition_counts_for_test();
            set_atomic_failpoint_for_test(Some(AtomicFailpoint::DurableMetadataAdmission));
            let applied = engine.apply_compiled_transaction(compiled, true);
            set_atomic_failpoint_for_test(None);
            let error = applied.expect_err("durable metadata failpoint must abort the commit");
            assert_eq!(atomic_audit(&engine), before);
            RangeAudit {
                error,
                cached_counts: take_cached_render_counts_for_test(),
                lifecycle_counts: take_localized_render_transition_counts_for_test(),
                full_pass_counts: take_full_pass_counts_for_test(),
            }
        }

        let generic = run(None);
        assert_eq!(generic.lifecycle_counts, (0, 0, 0));
        for (case, affected) in [
            ("empty", vec![]),
            ("tooNarrow", vec![1, 2]),
            ("wrongStart", vec![0, 1, 2]),
            ("duplicate", vec![1, 2, 2]),
            ("outOfOrder", vec![1, 3, 2]),
            ("outOfRange", vec![1, 2, 4]),
        ] {
            let malformed = run(Some(affected));
            assert_eq!(malformed.error, generic.error, "{case}");
            assert_eq!(malformed.cached_counts, generic.cached_counts, "{case}");
            assert_eq!(
                malformed.full_pass_counts, generic.full_pass_counts,
                "{case}"
            );
            assert_eq!(malformed.lifecycle_counts, (1, 0, 1), "{case}");
        }
    }

    #[test]
    fn every_localized_render_stage_failure_falls_back_with_exact_parity() {
        use crate::render::incremental::{
            reset_cached_render_counts_for_test,
            reset_localized_render_failure_checkpoint_counts_for_test,
            reset_localized_render_transition_counts_for_test,
            set_localized_render_failure_stage_for_test, take_cached_render_counts_for_test,
            take_localized_render_failure_checkpoint_counts_for_test,
            take_localized_render_transition_counts_for_test, LocalizedRenderFailureStage,
        };

        fn fixture() -> YrsDocumentEngine {
            let mut engine = transaction_engine();
            engine
                .import_json(
                    r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]},{"type":"paragraph","content":[{"type":"text","text":"suffix"}]}]}"#,
                    TransactionOrigin::DocumentImport,
                )
                .unwrap();
            hydrate_import_for_compile_test(&mut engine);
            engine
        }

        fn transaction(engine: &YrsDocumentEngine, request_id: u64) -> TypedTransaction {
            TypedTransaction {
                request_id,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: RevisionedPosition {
                        offset: 1,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    text: "x".into(),
                    marks: Vec::new(),
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            }
        }

        let mut generic = fixture();
        let mut generic_compiled = generic
            .compile_typed_transaction(transaction(&generic, 700_152))
            .unwrap();
        generic_compiled
            .prepared_derived_evidence
            .as_mut()
            .unwrap()
            .tamper_localized_render_for_test("missing");
        let generic_result = generic
            .apply_compiled_transaction(generic_compiled, true)
            .unwrap()
            .1
            .unwrap();

        for stage in [
            LocalizedRenderFailureStage::Allocation,
            LocalizedRenderFailureStage::Resource,
            LocalizedRenderFailureStage::Position,
            LocalizedRenderFailureStage::Invariant,
        ] {
            let mut engine = fixture();
            let compiled = engine
                .compile_typed_transaction(transaction(&engine, 700_152))
                .unwrap();
            reset_cached_render_counts_for_test();
            reset_localized_render_transition_counts_for_test();
            reset_localized_render_failure_checkpoint_counts_for_test();
            set_localized_render_failure_stage_for_test(Some(stage));
            let applied = engine.apply_compiled_transaction(compiled, true);
            set_localized_render_failure_stage_for_test(None);
            let result = applied.unwrap().1.unwrap();

            assert_eq!(result, generic_result, "{stage:?}");
            assert_eq!(engine.document_json(), generic.document_json(), "{stage:?}");
            let state = engine.derived_state.as_ref().unwrap();
            let generic_state = generic.derived_state.as_ref().unwrap();
            assert_eq!(
                state.validation_certificate, generic_state.validation_certificate,
                "{stage:?}"
            );
            assert_eq!(
                state.localized_text_index, generic_state.localized_text_index,
                "{stage:?}"
            );
            assert_eq!(
                state.render_blocks.materialize(),
                generic_state.render_blocks.materialize(),
                "{stage:?}"
            );
            assert_eq!(engine.can_undo(), generic.can_undo(), "{stage:?}");
            assert_eq!(engine.can_redo(), generic.can_redo(), "{stage:?}");
            assert_eq!(
                engine.history.retained_units(0).unwrap(),
                generic.history.retained_units(0).unwrap(),
                "{stage:?}"
            );
            assert_eq!(
                engine.history.replay_audit_for_test(),
                generic.history.replay_audit_for_test(),
                "{stage:?}"
            );
            assert_eq!(
                take_cached_render_counts_for_test(),
                (0, 1, 1, 0, 0),
                "{stage:?}"
            );
            assert_eq!(
                take_localized_render_transition_counts_for_test(),
                (1, 0, 1),
                "{stage:?}"
            );
            let expected_checkpoints = match stage {
                LocalizedRenderFailureStage::Allocation => (1, 0, 0, 0),
                LocalizedRenderFailureStage::Resource => (1, 1, 0, 0),
                LocalizedRenderFailureStage::Position => (1, 1, 1, 0),
                LocalizedRenderFailureStage::Invariant => (1, 1, 1, 1),
            };
            assert_eq!(
                take_localized_render_failure_checkpoint_counts_for_test(),
                expected_checkpoints,
                "{stage:?}"
            );
        }
    }

    #[test]
    fn localized_render_failure_exposes_only_the_generic_transition_error() {
        use crate::render::incremental::{
            reset_cached_render_counts_for_test, reset_localized_render_transition_counts_for_test,
            set_cached_render_error_for_test, set_localized_render_failure_stage_for_test,
            take_cached_render_counts_for_test, take_localized_render_transition_counts_for_test,
            CachedRenderError, LocalizedRenderFailureStage,
        };
        use crate::yrs_engine::observability::{
            reset_full_pass_counts_for_test, take_full_pass_counts_for_test, FullPassCounts,
        };

        struct FailureAudit {
            error: crate::yrs_engine::OperationError,
            cached_counts: (usize, usize, usize, usize, usize),
            lifecycle_counts: (usize, usize, usize),
            full_pass_counts: FullPassCounts,
        }

        fn run(stage: Option<LocalizedRenderFailureStage>) -> FailureAudit {
            let mut engine = transaction_engine();
            engine
                .import_json(
                    r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]},{"type":"paragraph","content":[{"type":"text","text":"suffix"}]}]}"#,
                    TransactionOrigin::DocumentImport,
                )
                .unwrap();
            let mut compiled = engine
                .compile_typed_transaction(TypedTransaction {
                    request_id: 700_153,
                    base_document_revision: engine.revision(),
                    origin: TransactionOrigin::LocalInput,
                    operations: vec![TypedOperation::InsertText {
                        at: RevisionedPosition {
                            offset: 1,
                            kind: EditorOffsetKind::Scalar,
                            affinity: Affinity::After,
                        },
                        text: "x".into(),
                        marks: Vec::new(),
                    }],
                    selection_intent: SelectionIntent::UseOperationResult,
                    history_policy: HistoryPolicy::Auto,
                })
                .unwrap();
            compiled.localized_semantic_used = stage.is_some();
            let before = atomic_audit(&engine);
            reset_full_pass_counts_for_test();
            reset_cached_render_counts_for_test();
            reset_localized_render_transition_counts_for_test();
            set_localized_render_failure_stage_for_test(stage);
            set_cached_render_error_for_test(Some(CachedRenderError::AllocationFailed));
            let applied = engine.apply_compiled_transaction(compiled, true);
            set_localized_render_failure_stage_for_test(None);
            set_cached_render_error_for_test(None);
            let error = applied.expect_err("forced generic transition failure must be returned");
            assert_eq!(atomic_audit(&engine), before);
            FailureAudit {
                error,
                cached_counts: take_cached_render_counts_for_test(),
                lifecycle_counts: take_localized_render_transition_counts_for_test(),
                full_pass_counts: take_full_pass_counts_for_test(),
            }
        }

        let generic = run(None);
        assert_eq!(generic.error.code, "ENGINE_INVARIANT_FAILED");
        assert!(generic.error.message.contains("AllocationFailed"));
        assert_eq!(generic.cached_counts, (0, 1, 0, 0, 0));
        assert_eq!(generic.lifecycle_counts, (0, 0, 0));
        assert_eq!(generic.full_pass_counts, FullPassCounts::default());
        for stage in [
            LocalizedRenderFailureStage::Allocation,
            LocalizedRenderFailureStage::Resource,
            LocalizedRenderFailureStage::Position,
            LocalizedRenderFailureStage::Invariant,
        ] {
            let localized = run(Some(stage));
            assert_eq!(localized.error, generic.error, "{stage:?}");
            assert_eq!(localized.cached_counts, generic.cached_counts, "{stage:?}");
            assert_eq!(localized.lifecycle_counts, (1, 0, 1), "{stage:?}");
            assert_eq!(
                localized.full_pass_counts, generic.full_pass_counts,
                "{stage:?}"
            );
        }
    }

    #[test]
    fn changed_commit_survives_optional_index_allocation_failure_exactly() {
        use crate::yrs_engine::derived_state::force_localized_index_allocation_failure_for_test;

        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let before_revision = engine.revision();
        let before_state_revision = engine.state_revision();
        let point = RevisionedPosition {
            offset: 1,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::After,
        };
        force_localized_index_allocation_failure_for_test(true);
        let applied = engine.apply_typed_transaction_with_result(TypedTransaction {
            request_id: 700_121,
            base_document_revision: before_revision,
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertText {
                at: point,
                text: "x".into(),
                marks: Vec::new(),
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        });
        force_localized_index_allocation_failure_for_test(false);
        let result = applied.expect("optional index failure cannot abort commit");
        assert!(result.changed);
        assert_eq!(result.document_revision, before_revision + 1);
        assert_eq!(result.state_revision, before_state_revision + 1);
        assert!(result.changed);
        assert!(matches!(
            result.selection,
            crate::yrs_engine::ResolvedSelection::Text { ref anchor, ref head }
                if anchor.document == 3 && head.document == 3
        ));
        assert_eq!(
            engine.document_json().unwrap()["content"][0]["content"][0]["text"],
            "axbc"
        );
        assert!(engine.can_undo());
        let state = engine.derived_state.as_ref().unwrap();
        assert_eq!(state.document_revision, result.document_revision);
        assert_eq!(state.state_revision, result.state_revision);
        assert!(state.localized_text_index.is_none());

        let compiled = engine
            .compile_typed_transaction(TypedTransaction {
                request_id: 700_122,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: RevisionedPosition { offset: 2, ..point },
                    text: "y".into(),
                    marks: Vec::new(),
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            })
            .unwrap();
        assert!(compiled.localized_insert_admission.is_none());
    }

    #[test]
    fn changed_commit_survives_optional_index_budget_failure_exactly() {
        use crate::yrs_engine::derived_state::force_localized_index_budget_for_test;

        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let before_revision = engine.revision();
        force_localized_index_budget_for_test(Some(1));
        let result = engine.apply_typed_transaction_with_result(TypedTransaction {
            request_id: 700_123,
            base_document_revision: before_revision,
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertText {
                at: RevisionedPosition {
                    offset: 1,
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::After,
                },
                text: "x".into(),
                marks: Vec::new(),
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        });
        force_localized_index_budget_for_test(None);
        let result = result.expect("optional index budget cannot abort commit");
        assert!(result.changed);
        assert_eq!(result.document_revision, before_revision + 1);
        assert_eq!(
            engine.document_json().unwrap()["content"][0]["content"][0]["text"],
            "axbc"
        );
        assert!(engine.can_undo());
        assert!(engine
            .derived_state
            .as_ref()
            .unwrap()
            .localized_text_index
            .is_none());
    }

    #[test]
    fn changed_commit_survives_each_optional_index_allocation_stage() {
        use crate::yrs_engine::derived_state::{
            force_localized_index_allocation_stage_for_test, LocalizedIndexAllocationStage,
        };

        for (stage_index, stage) in [
            LocalizedIndexAllocationStage::InitialLeafCapacity,
            LocalizedIndexAllocationStage::TraversalPath,
            LocalizedIndexAllocationStage::LeafGrowth,
        ]
        .into_iter()
        .enumerate()
        {
            let mut engine = transaction_engine();
            engine
                .import_json(
                    r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"ab"},{"type":"hardBreak"},{"type":"text","text":"cd"}]}]}"#,
                    TransactionOrigin::DocumentImport,
                )
                .unwrap();
            let before_document_revision = engine.revision();
            let before_state_revision = engine.state_revision();
            let point = RevisionedPosition {
                offset: 1,
                kind: EditorOffsetKind::Scalar,
                affinity: Affinity::After,
            };
            force_localized_index_allocation_stage_for_test(Some(stage));
            let applied = engine.apply_typed_transaction_with_result(TypedTransaction {
                request_id: 700_130 + u64::try_from(stage_index).unwrap(),
                base_document_revision: before_document_revision,
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: point,
                    text: "x".into(),
                    marks: Vec::new(),
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            });
            force_localized_index_allocation_stage_for_test(None);

            let result = applied.expect("optional index failure cannot abort a live commit");
            assert!(result.changed, "stage {stage:?}");
            assert_eq!(result.document_revision, before_document_revision + 1);
            assert_eq!(result.state_revision, before_state_revision + 1);
            assert_eq!(
                engine.document_json().unwrap()["content"][0]["content"][0]["text"],
                "axb"
            );
            assert!(engine.can_undo());
            let state = engine.derived_state.as_ref().unwrap();
            assert_eq!(state.document_revision, result.document_revision);
            assert_eq!(state.state_revision, result.state_revision);
            assert!(state.localized_text_index.is_none(), "stage {stage:?}");

            let compiled = engine
                .compile_typed_transaction(TypedTransaction {
                    request_id: 700_140 + u64::try_from(stage_index).unwrap(),
                    base_document_revision: engine.revision(),
                    origin: TransactionOrigin::LocalInput,
                    operations: vec![TypedOperation::InsertText {
                        at: RevisionedPosition { offset: 2, ..point },
                        text: "y".into(),
                        marks: Vec::new(),
                    }],
                    selection_intent: SelectionIntent::UseOperationResult,
                    history_policy: HistoryPolicy::Auto,
                })
                .unwrap();
            assert!(compiled.localized_insert_admission.is_none());
        }
    }

    #[test]
    fn selection_only_optional_index_copy_failure_degrades_evidence_to_none() {
        use crate::yrs_engine::derived_state::{
            force_localized_index_allocation_stage_for_test, LocalizedIndexAllocationStage,
        };

        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let before_document_revision = engine.revision();
        let before_state_revision = engine.state_revision();
        let point = RevisionedPosition {
            offset: 1,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::After,
        };
        force_localized_index_allocation_stage_for_test(Some(
            LocalizedIndexAllocationStage::InitialLeafCapacity,
        ));
        assert!(engine
            .derived_state
            .as_ref()
            .unwrap()
            .clone_with_fallible_localized_index()
            .localized_text_index
            .is_none());
        let applied = engine.apply_typed_transaction_with_result(TypedTransaction {
            request_id: 700_150,
            base_document_revision: before_document_revision,
            origin: TransactionOrigin::LocalApi,
            operations: Vec::new(),
            selection_intent: SelectionIntent::Set(SelectionInput::Text {
                anchor: point,
                head: point,
            }),
            history_policy: HistoryPolicy::Auto,
        });
        force_localized_index_allocation_stage_for_test(None);

        let result = applied.expect("optional evidence copy failure cannot abort selection");
        assert!(result.changed);
        assert_eq!(result.document_revision, before_document_revision);
        assert_eq!(result.state_revision, before_state_revision + 1);
        let state = engine.derived_state.as_ref().unwrap();
        assert!(state.localized_text_index.is_none());
        assert_eq!(state.document_revision, before_document_revision);
        assert_eq!(state.state_revision, before_state_revision + 1);

        let compiled = engine
            .compile_typed_transaction(TypedTransaction {
                request_id: 700_151,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: point,
                    text: "x".into(),
                    marks: Vec::new(),
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            })
            .unwrap();
        assert!(compiled.localized_insert_admission.is_none());
    }

    #[test]
    fn selection_only_revision_reseal_allows_following_strict_insert_admission() {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let point = RevisionedPosition {
            offset: 1,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::After,
        };
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 70_014,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: Vec::new(),
                selection_intent: SelectionIntent::Set(SelectionInput::Text {
                    anchor: point,
                    head: point,
                }),
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap();
        hydrate_import_for_compile_test(&mut engine);
        let state = engine.derived_state.as_ref().unwrap();
        assert_eq!(
            state.validation_certificate.state_revision(),
            engine.state_revision()
        );

        let compiled = engine
            .compile_typed_transaction(TypedTransaction {
                request_id: 70_015,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: point,
                    text: "x".into(),
                    marks: Vec::new(),
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            })
            .unwrap();
        assert!(compiled.localized_insert_admission.is_some());

        engine
            .apply_command(
                70_016,
                TypedCommand::ToggleMark {
                    mark_type: "bold".into(),
                },
            )
            .unwrap()
            .unwrap();
        engine
            .apply_command(
                700_161,
                TypedCommand::ToggleMark {
                    mark_type: "bold".into(),
                },
            )
            .unwrap()
            .unwrap();
        let state = engine.derived_state.as_ref().unwrap();
        assert_eq!(
            state.validation_certificate.state_revision(),
            engine.state_revision()
        );
        let stored_marks = engine.stored_marks().unwrap_or_default().to_vec();
        let compiled = engine
            .compile_typed_transaction(TypedTransaction {
                request_id: 70_017,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: point,
                    text: "x".into(),
                    marks: stored_marks,
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            })
            .unwrap();
        assert!(compiled.localized_insert_admission.is_some());
    }

    #[test]
    fn benchmark_shaped_bursts_decompose_direct_result_and_command_full_passes() {
        use crate::render::incremental::{
            reset_cached_render_counts_for_test, reset_localized_render_transition_counts_for_test,
            take_cached_render_counts_for_test, take_localized_render_transition_counts_for_test,
        };
        use crate::yrs_engine::derived_state::{
            reset_active_state_cache_counts_for_test,
            reset_localized_index_lifecycle_counts_for_test,
            take_active_state_cache_counts_for_test,
            take_localized_index_lifecycle_counts_for_test,
        };
        use crate::yrs_engine::mutation::{
            reset_localized_lookup_counts_for_test, take_localized_lookup_counts_for_test,
        };
        use crate::yrs_engine::observability::{
            reset_full_pass_counts_for_test, take_full_pass_counts_for_test, FullPassCounts,
        };

        fn fixture() -> YrsDocumentEngine {
            let mut engine = transaction_engine();
            let content = (0..160)
                .map(|index| {
                    json!({
                        "type": "paragraph",
                        "content": [{
                            "type": "text",
                            "text": format!("{index:04} {}", "x".repeat(214))
                        }]
                    })
                })
                .collect::<Vec<_>>();
            engine
                .import_json(
                    &json!({"type": "doc", "content": content}).to_string(),
                    TransactionOrigin::DocumentImport,
                )
                .unwrap();
            let point = RevisionedPosition {
                offset: 44,
                kind: EditorOffsetKind::Scalar,
                affinity: Affinity::After,
            };
            engine
                .apply_typed_transaction(TypedTransaction {
                    request_id: 70_100,
                    base_document_revision: engine.revision(),
                    origin: TransactionOrigin::LocalApi,
                    operations: Vec::new(),
                    selection_intent: SelectionIntent::Set(SelectionInput::Text {
                        anchor: point,
                        head: point,
                    }),
                    history_policy: HistoryPolicy::Skip,
                })
                .unwrap();
            hydrate_import_for_compile_test(&mut engine);
            engine
        }

        fn direct(engine: &YrsDocumentEngine, index: usize) -> TypedTransaction {
            TypedTransaction {
                request_id: 70_200 + index as u64,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: RevisionedPosition {
                        offset: 44 + index as u32,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    text: "x".into(),
                    marks: Vec::new(),
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            }
        }

        let mut direct_commit = fixture();
        let mut commit_counts = Vec::new();
        for index in 0..20 {
            reset_full_pass_counts_for_test();
            reset_localized_lookup_counts_for_test();
            reset_localized_index_lifecycle_counts_for_test();
            reset_cached_render_counts_for_test();
            reset_localized_render_transition_counts_for_test();
            let transaction = direct(&direct_commit, index);
            direct_commit.apply_typed_transaction(transaction).unwrap();
            commit_counts.push((
                take_full_pass_counts_for_test(),
                take_localized_lookup_counts_for_test(),
                take_localized_index_lifecycle_counts_for_test(),
                take_cached_render_counts_for_test(),
                take_localized_render_transition_counts_for_test(),
            ));
        }

        let mut direct_result = fixture();
        let mut result_counts = Vec::new();
        for index in 0..20 {
            reset_full_pass_counts_for_test();
            reset_localized_lookup_counts_for_test();
            reset_localized_index_lifecycle_counts_for_test();
            reset_cached_render_counts_for_test();
            reset_localized_render_transition_counts_for_test();
            let transaction = direct(&direct_result, index);
            direct_result
                .apply_typed_transaction_with_result(transaction)
                .unwrap();
            result_counts.push((
                take_full_pass_counts_for_test(),
                take_localized_lookup_counts_for_test(),
                take_localized_index_lifecycle_counts_for_test(),
                take_cached_render_counts_for_test(),
                take_localized_render_transition_counts_for_test(),
            ));
        }

        let mut command = fixture();
        let mut command_counts = Vec::new();
        reset_active_state_cache_counts_for_test();
        for index in 0..20 {
            reset_full_pass_counts_for_test();
            reset_localized_lookup_counts_for_test();
            reset_localized_index_lifecycle_counts_for_test();
            reset_cached_render_counts_for_test();
            reset_localized_render_transition_counts_for_test();
            command
                .apply_command(
                    70_300 + index as u64,
                    TypedCommand::InsertText { text: "x".into() },
                )
                .unwrap()
                .unwrap();
            command_counts.push((
                take_full_pass_counts_for_test(),
                take_localized_lookup_counts_for_test(),
                take_localized_index_lifecycle_counts_for_test(),
                take_cached_render_counts_for_test(),
                take_localized_render_transition_counts_for_test(),
            ));
        }
        assert_eq!(
            take_active_state_cache_counts_for_test(),
            (20, 19, 1, 1, 20, 20, 0, 20, 1),
            "prepared command burst must build ActiveState once, then reuse it"
        );

        let expected_commit = (
            FullPassCounts {
                import_model_parses: 0,
                validated_evidence_constructions: 0,
                validation_certificate_constructions: 0,
                planner_simulations: 0,
                document_validations: 0,
                canonical_mark_tree_scans: 0,
                canonical_mark_validation_attempts: 0,
                canonical_mark_validation_completions: 0,
                canonical_mark_nodes_visited: 0,
                canonical_identity_predicate_nodes_visited: 0,
                canonical_projections: 1,
                canonical_serializations: 2,
                canonical_hashes: 1,
                affected_top_level_scans: 0,
                position_map_clones: 1,
                position_map_compactions: 1,
                rendered_text_derivations: 0,
                raw_document_text_scans: 0,
                document_node_count_scans: 0,
                render_limit_tree_scans: 0,
                render_identity_scans: 0,
                render_top_level_start_scans: 0,
                active_applicability_passes: 0,
                ordinary_step_applications: 0,
            },
            (0, 1, 1),
            (0, 1, 1, 0),
            (0, 1, 1, 0, 0),
            (1, 1, 0),
        );
        let expected_result = (
            FullPassCounts {
                import_model_parses: 0,
                validated_evidence_constructions: 0,
                validation_certificate_constructions: 0,
                planner_simulations: 0,
                document_validations: 0,
                canonical_mark_tree_scans: 0,
                canonical_mark_validation_attempts: 0,
                canonical_mark_validation_completions: 0,
                canonical_mark_nodes_visited: 0,
                canonical_identity_predicate_nodes_visited: 0,
                canonical_projections: 1,
                canonical_serializations: 2,
                canonical_hashes: 1,
                affected_top_level_scans: 0,
                position_map_clones: 1,
                position_map_compactions: 1,
                rendered_text_derivations: 0,
                raw_document_text_scans: 0,
                document_node_count_scans: 0,
                render_limit_tree_scans: 0,
                render_identity_scans: 0,
                render_top_level_start_scans: 0,
                active_applicability_passes: 1,
                ordinary_step_applications: 0,
            },
            (0, 1, 1),
            (0, 1, 1, 0),
            (0, 1, 1, 0, 0),
            (1, 1, 0),
        );
        let expected_command = (
            FullPassCounts {
                import_model_parses: 0,
                validated_evidence_constructions: 0,
                validation_certificate_constructions: 0,
                planner_simulations: 1,
                document_validations: 1,
                canonical_mark_tree_scans: 0,
                canonical_mark_validation_attempts: 0,
                canonical_mark_validation_completions: 0,
                canonical_mark_nodes_visited: 0,
                canonical_identity_predicate_nodes_visited: 321,
                canonical_projections: 1,
                canonical_serializations: 1,
                canonical_hashes: 1,
                affected_top_level_scans: 0,
                position_map_clones: 1,
                position_map_compactions: 1,
                rendered_text_derivations: 0,
                raw_document_text_scans: 1,
                document_node_count_scans: 0,
                render_limit_tree_scans: 0,
                render_identity_scans: 0,
                render_top_level_start_scans: 0,
                active_applicability_passes: 1,
                ordinary_step_applications: 1,
            },
            (0, 1, 1),
            (0, 1, 1, 0),
            (0, 1, 1, 0, 0),
            (1, 1, 0),
        );
        for (index, actual) in commit_counts.iter().enumerate() {
            assert_eq!(*actual, expected_commit, "direct commit edit {index}");
        }
        for (index, actual) in result_counts.iter().enumerate() {
            assert_eq!(*actual, expected_result, "direct result edit {index}");
        }
        for (index, actual) in command_counts.iter().enumerate() {
            let mut expected = expected_command;
            expected.0.active_applicability_passes = usize::from(index == 0);
            assert_eq!(*actual, expected, "command edit {index}");
        }

        let mut promoted = fixture();
        let mut rebuilt = fixture();
        for index in 0..20 {
            rebuilt.derived_state.as_mut().unwrap().localized_text_index = None;
            let promoted_transaction = direct(&promoted, index);
            let rebuilt_transaction = direct(&rebuilt, index);
            let promoted_result = promoted
                .apply_typed_transaction_with_result(promoted_transaction)
                .unwrap();
            let rebuilt_result = rebuilt
                .apply_typed_transaction_with_result(rebuilt_transaction)
                .unwrap();
            assert_eq!(promoted_result, rebuilt_result, "sequential edit {index}");
            assert_eq!(promoted.document_json(), rebuilt.document_json());
            let promoted_state = promoted.derived_state.as_ref().unwrap();
            let rebuilt_state = rebuilt.derived_state.as_ref().unwrap();
            assert_eq!(
                promoted_state.validation_certificate, rebuilt_state.validation_certificate,
                "sequential edit {index}"
            );
            assert_eq!(
                promoted_state.localized_text_index, rebuilt_state.localized_text_index,
                "sequential edit {index}"
            );
        }
        assert_eq!(
            promoted.undo(700_153).unwrap(),
            rebuilt.undo(700_153).unwrap()
        );
        assert_eq!(promoted.document_json(), rebuilt.document_json());
        assert_eq!(
            promoted.redo(700_154).unwrap(),
            rebuilt.redo(700_154).unwrap()
        );
        assert_eq!(promoted.document_json(), rebuilt.document_json());
    }

    #[test]
    fn prepared_active_state_cache_allocation_and_budget_misses_are_optional() {
        use crate::yrs_engine::derived_state::{
            force_active_state_cache_allocation_failure_for_test,
            force_active_state_cache_budget_for_test,
            force_active_state_public_materialization_failure_for_test,
            reset_active_state_cache_counts_for_test, take_active_state_cache_counts_for_test,
        };

        fn fixture() -> YrsDocumentEngine {
            let mut engine = transaction_engine();
            engine
                .import_json(
                    r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                    TransactionOrigin::DocumentImport,
                )
                .unwrap();
            let point = RevisionedPosition {
                offset: 1,
                kind: EditorOffsetKind::Scalar,
                affinity: Affinity::After,
            };
            engine
                .apply_typed_transaction(TypedTransaction {
                    request_id: 710_000,
                    base_document_revision: engine.revision(),
                    origin: TransactionOrigin::LocalApi,
                    operations: Vec::new(),
                    selection_intent: SelectionIntent::Set(SelectionInput::Text {
                        anchor: point,
                        head: point,
                    }),
                    history_policy: HistoryPolicy::Skip,
                })
                .unwrap();
            engine
        }

        for budget_failure in [false, true] {
            let mut engine = fixture();
            reset_active_state_cache_counts_for_test();
            if budget_failure {
                force_active_state_cache_budget_for_test(Some(0));
            } else {
                force_active_state_cache_allocation_failure_for_test(true);
            }
            let result = engine
                .apply_command(710_001, TypedCommand::InsertText { text: "x".into() })
                .unwrap()
                .unwrap();
            force_active_state_cache_budget_for_test(None);
            force_active_state_cache_allocation_failure_for_test(false);

            assert!(result.changed);
            assert_eq!(
                engine.document_json().unwrap()["content"][0]["content"][0]["text"],
                "axbc"
            );
            assert!(engine.can_undo());
            assert!(engine
                .derived_state
                .as_ref()
                .unwrap()
                .active_state_cache_for_test()
                .is_none());
            assert_eq!(
                take_active_state_cache_counts_for_test(),
                (1, 0, 1, 1, 1, 0, 0, 0, 1)
            );
        }

        let mut measured = fixture();
        let measured_result = measured
            .apply_command(710_010, TypedCommand::InsertText { text: "x".into() })
            .unwrap()
            .unwrap();
        let retained = measured
            .derived_state
            .as_ref()
            .unwrap()
            .active_state_cache_for_test()
            .unwrap()
            .retained_bytes_for_test();

        let mut exact = fixture();
        reset_active_state_cache_counts_for_test();
        force_active_state_cache_budget_for_test(Some(retained));
        let exact_result = exact
            .apply_command(710_010, TypedCommand::InsertText { text: "x".into() })
            .unwrap()
            .unwrap();
        force_active_state_cache_budget_for_test(None);
        assert_eq!(exact_result, measured_result);
        assert_eq!(
            take_active_state_cache_counts_for_test(),
            (1, 0, 1, 1, 1, 1, 0, 1, 1)
        );

        let mut under = fixture();
        reset_active_state_cache_counts_for_test();
        force_active_state_cache_budget_for_test(Some(retained - 1));
        let under_result = under
            .apply_command(710_010, TypedCommand::InsertText { text: "x".into() })
            .unwrap()
            .unwrap();
        force_active_state_cache_budget_for_test(None);
        assert_eq!(under_result, measured_result);
        assert_eq!(under.document_json(), measured.document_json());
        assert!(under
            .derived_state
            .as_ref()
            .unwrap()
            .active_state_cache_for_test()
            .is_none());
        assert_eq!(
            take_active_state_cache_counts_for_test(),
            (1, 0, 1, 1, 1, 0, 0, 0, 1)
        );

        let mut materialization = measured;
        let mut baseline = exact;
        reset_active_state_cache_counts_for_test();
        force_active_state_public_materialization_failure_for_test(true);
        let materialized_result =
            materialization.apply_command(710_011, TypedCommand::InsertText { text: "y".into() });
        force_active_state_public_materialization_failure_for_test(false);
        let materialized_result = materialized_result.unwrap().unwrap();
        assert_eq!(
            take_active_state_cache_counts_for_test(),
            (1, 0, 1, 1, 1, 0, 1, 0, 1)
        );
        let baseline_result = baseline
            .apply_command(710_011, TypedCommand::InsertText { text: "y".into() })
            .unwrap()
            .unwrap();
        assert_eq!(materialized_result, baseline_result);
        assert_eq!(materialization.document_json(), baseline.document_json());
        assert_eq!(materialization.can_undo(), baseline.can_undo());
        assert_eq!(materialization.can_redo(), baseline.can_redo());
        assert!(materialization
            .derived_state
            .as_ref()
            .unwrap()
            .active_state_cache_for_test()
            .is_none());
    }

    #[test]
    fn prepared_active_state_transition_tamper_falls_back_with_exact_parity() {
        use crate::yrs_engine::derived_state::{
            reset_active_state_cache_counts_for_test, take_active_state_cache_counts_for_test,
        };

        fn fixture() -> YrsDocumentEngine {
            let mut engine = transaction_engine();
            engine
                .import_json(
                    r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                    TransactionOrigin::DocumentImport,
                )
                .unwrap();
            let point = RevisionedPosition {
                offset: 1,
                kind: EditorOffsetKind::Scalar,
                affinity: Affinity::After,
            };
            engine
                .apply_typed_transaction(TypedTransaction {
                    request_id: 711_000,
                    base_document_revision: engine.revision(),
                    origin: TransactionOrigin::LocalApi,
                    operations: Vec::new(),
                    selection_intent: SelectionIntent::Set(SelectionInput::Text {
                        anchor: point,
                        head: point,
                    }),
                    history_policy: HistoryPolicy::Skip,
                })
                .unwrap();
            engine
                .apply_command(711_001, TypedCommand::InsertText { text: "x".into() })
                .unwrap()
                .unwrap();
            engine
        }

        fn compiled_insert(engine: &YrsDocumentEngine, request_id: u64) -> CompiledTransaction {
            let preparation = std::cell::RefCell::new(None);
            let CommandPlan::Transaction(transaction) = engine
                .plan_command_internal(
                    request_id,
                    TypedCommand::InsertText { text: "y".into() },
                    Some(&preparation),
                )
                .unwrap()
            else {
                panic!("insert command must prepare a transaction")
            };
            engine
                .compile_prepared_typed_transaction(transaction, preparation.into_inner().unwrap())
                .unwrap()
        }

        for (index, claim) in [
            "documentRevision",
            "stateRevision",
            "epoch",
            "schema",
            "resource",
            "editing",
            "maxLength",
            "selection",
            "relativeSelection",
            "legacySelection",
            "storedMarks",
            "structural",
            "resultSelection",
            "preview",
            "render",
            "lookup",
            "validation",
            "cachedPayloadIdentity",
        ]
        .into_iter()
        .enumerate()
        {
            let mut tampered = fixture();
            let mut generic = fixture();
            let request_id = 711_100 + u64::try_from(index).unwrap();
            let mut tampered_compiled = compiled_insert(&tampered, request_id);
            tampered_compiled
                .prepared_active_state_transition
                .as_mut()
                .unwrap()
                .tamper_for_test(claim);
            let mut generic_compiled = compiled_insert(&generic, request_id);
            generic_compiled.prepared_active_state_transition = None;

            reset_active_state_cache_counts_for_test();
            let tampered_result = tampered
                .apply_compiled_transaction(tampered_compiled, true)
                .unwrap()
                .1
                .unwrap();
            assert_eq!(
                take_active_state_cache_counts_for_test(),
                (1, 0, 1, 1, 0, 0, 1, 0, 1),
                "{claim}"
            );
            let generic_result = generic
                .apply_compiled_transaction(generic_compiled, true)
                .unwrap()
                .1
                .unwrap();
            assert_eq!(tampered_result, generic_result, "{claim}");
            assert_eq!(tampered.document_json(), generic.document_json(), "{claim}");
            assert_eq!(tampered.can_undo(), generic.can_undo(), "{claim}");
            assert_eq!(tampered.can_redo(), generic.can_redo(), "{claim}");
            assert!(tampered
                .derived_state
                .as_ref()
                .unwrap()
                .active_state_cache_for_test()
                .is_none());
        }

        for (index, current_claim) in [
            "missingCurrentCertificate",
            "replacedCurrentCertificate",
            "replacedCurrentPayload",
        ]
        .into_iter()
        .enumerate()
        {
            let mut engine = fixture();
            let compiled = compiled_insert(&engine, 711_500 + u64::try_from(index).unwrap());
            let state = engine.derived_state.as_mut().unwrap();
            match current_claim {
                "missingCurrentCertificate" => state.remove_active_state_certificate_for_test(),
                "replacedCurrentCertificate" => {
                    state.replace_active_state_certificate_identity_for_test()
                }
                "replacedCurrentPayload" => state.replace_active_state_payload_identity_for_test(),
                _ => unreachable!(),
            }
            reset_active_state_cache_counts_for_test();
            let result = engine
                .apply_compiled_transaction(compiled, true)
                .unwrap()
                .1
                .unwrap();
            assert!(result.changed, "{current_claim}");
            let expected_drops = usize::from(current_claim != "missingCurrentCertificate");
            assert_eq!(
                take_active_state_cache_counts_for_test(),
                (1, 0, 1, 1, 0, 0, expected_drops, 0, 1),
                "{current_claim}"
            );
            assert!(engine
                .derived_state
                .as_ref()
                .unwrap()
                .active_state_cache_for_test()
                .is_none());
        }
    }

    #[test]
    fn prepared_active_state_cache_survives_post_result_rejection_by_identity() {
        use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};

        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let point = RevisionedPosition {
            offset: 1,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::After,
        };
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 712_000,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: Vec::new(),
                selection_intent: SelectionIntent::Set(SelectionInput::Text {
                    anchor: point,
                    head: point,
                }),
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap();
        engine
            .apply_command(712_001, TypedCommand::InsertText { text: "x".into() })
            .unwrap()
            .unwrap();
        let before = atomic_audit(&engine);
        let cache_before = engine
            .derived_state
            .as_ref()
            .unwrap()
            .active_state_cache_for_test()
            .unwrap();

        let preparation = std::cell::RefCell::new(None);
        let CommandPlan::Transaction(transaction) = engine
            .plan_command_internal(
                712_002,
                TypedCommand::InsertText { text: "y".into() },
                Some(&preparation),
            )
            .unwrap()
        else {
            panic!("insert command must prepare a transaction")
        };
        let compiled = engine
            .compile_prepared_typed_transaction(transaction, preparation.into_inner().unwrap())
            .unwrap();
        set_atomic_failpoint_for_test(Some(AtomicFailpoint::FinalPreflight));
        let rejected = engine.apply_compiled_transaction(compiled, true);
        set_atomic_failpoint_for_test(None);
        assert!(rejected.is_err());
        assert_eq!(atomic_audit(&engine), before);
        let cache_after = engine
            .derived_state
            .as_ref()
            .unwrap()
            .active_state_cache_for_test()
            .unwrap();
        assert!(Arc::ptr_eq(&cache_before, &cache_after));
    }

    #[test]
    fn prepared_active_state_certificate_is_cleared_by_changed_state_boundaries() {
        fn fixture() -> YrsDocumentEngine {
            let mut engine = transaction_engine();
            engine
                .import_json(
                    r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                    TransactionOrigin::DocumentImport,
                )
                .unwrap();
            let point = RevisionedPosition {
                offset: 1,
                kind: EditorOffsetKind::Scalar,
                affinity: Affinity::After,
            };
            engine
                .apply_typed_transaction(TypedTransaction {
                    request_id: 713_000,
                    base_document_revision: engine.revision(),
                    origin: TransactionOrigin::LocalApi,
                    operations: Vec::new(),
                    selection_intent: SelectionIntent::Set(SelectionInput::Text {
                        anchor: point,
                        head: point,
                    }),
                    history_policy: HistoryPolicy::Skip,
                })
                .unwrap();
            engine
                .apply_command(713_001, TypedCommand::InsertText { text: "x".into() })
                .unwrap()
                .unwrap();
            assert!(engine
                .derived_state
                .as_ref()
                .unwrap()
                .active_state_cache_for_test()
                .is_some());
            engine
        }

        let assert_cleared = |engine: &YrsDocumentEngine, boundary: &str| {
            assert!(
                engine
                    .derived_state
                    .as_ref()
                    .unwrap()
                    .active_state_cache_for_test()
                    .is_none(),
                "{boundary}"
            );
        };

        let mut selection = fixture();
        let point = RevisionedPosition {
            offset: 0,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::After,
        };
        selection
            .apply_typed_transaction(TypedTransaction {
                request_id: 713_010,
                base_document_revision: selection.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: Vec::new(),
                selection_intent: SelectionIntent::Set(SelectionInput::Text {
                    anchor: point,
                    head: point,
                }),
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap();
        assert_cleared(&selection, "selection");

        let mut direct = fixture();
        let caret = direct
            .derived_state
            .as_ref()
            .unwrap()
            .resolved_selection
            .clone();
        let crate::yrs_engine::ResolvedSelection::Text { anchor, .. } = caret else {
            panic!("fixture retains a text caret")
        };
        direct
            .apply_typed_transaction_with_result(TypedTransaction {
                request_id: 713_011,
                base_document_revision: direct.revision(),
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: RevisionedPosition {
                        offset: anchor.document,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    text: "y".into(),
                    marks: Vec::new(),
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            })
            .unwrap();
        assert_cleared(&direct, "direct LocalInput");

        let mut undone = fixture();
        undone.undo(713_012).unwrap();
        assert_cleared(&undone, "undo");
        undone.redo(713_013).unwrap();
        assert_cleared(&undone, "redo");

        let mut stored_mark = fixture();
        stored_mark
            .apply_command(
                713_014,
                TypedCommand::ToggleMark {
                    mark_type: "bold".into(),
                },
            )
            .unwrap()
            .unwrap();
        assert_cleared(&stored_mark, "stored mark");

        let mut deleted = fixture();
        deleted
            .apply_command(713_015, TypedCommand::DeleteBackward)
            .unwrap()
            .unwrap();
        assert_cleared(&deleted, "prepared delete");

        let mut structural = fixture();
        structural
            .apply_command(713_016, TypedCommand::ToggleHeading { level: 2 })
            .unwrap()
            .unwrap();
        assert_cleared(&structural, "prepared structural command");

        let mut no_result = fixture();
        let crate::yrs_engine::ResolvedSelection::Text { anchor, .. } = no_result
            .derived_state
            .as_ref()
            .unwrap()
            .resolved_selection
            .clone()
        else {
            panic!("fixture retains a text caret")
        };
        no_result
            .apply_typed_transaction(TypedTransaction {
                request_id: 713_017,
                base_document_revision: no_result.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: vec![TypedOperation::InsertText {
                    at: RevisionedPosition {
                        offset: anchor.document,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    text: "z".into(),
                    marks: Vec::new(),
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            })
            .unwrap();
        assert_cleared(&no_result, "no-result changed transaction");

        let mut imported = fixture();
        imported
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"replacement"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        assert_cleared(&imported, "import");

        let mut source = transaction_engine();
        source
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"snapshot"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let snapshot = source.export_snapshot().unwrap();
        let mut restored = fixture();
        restored.restore_snapshot(&snapshot).unwrap();
        assert_cleared(&restored, "snapshot restore");

        let mut source = transaction_engine();
        source
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let mut remote = YrsDocumentEngine::new(YrsEngineConfig {
            schema: tiptap_schema(),
            fragment_name: "prosemirror".into(),
            initialization_mode: crate::yrs_engine::InitializationMode::AwaitRemote,
            resource_limits: ResourceLimits::default(),
            editing_limits: crate::yrs_engine::EditingLimits::default(),
            max_length: None,
            scope: Some(crate::yrs_engine::DocumentScope {
                document_id: "active-cache".into(),
                lineage_id: "invalidation".into(),
            }),
        })
        .unwrap();
        remote
            .apply_remote_update_v1(713_020, &source.encoded_state().unwrap())
            .unwrap();
        let point = RevisionedPosition {
            offset: 1,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::After,
        };
        remote
            .apply_typed_transaction(TypedTransaction {
                request_id: 713_021,
                base_document_revision: remote.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: Vec::new(),
                selection_intent: SelectionIntent::Set(SelectionInput::Text {
                    anchor: point,
                    head: point,
                }),
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap();
        remote
            .apply_command(713_022, TypedCommand::InsertText { text: "x".into() })
            .unwrap()
            .unwrap();
        assert!(remote
            .derived_state
            .as_ref()
            .unwrap()
            .active_state_cache_for_test()
            .is_some());
        source
            .apply_typed_transaction(insert_transaction(&source, 713_023))
            .unwrap();
        let remote_vector = remote.doc.transact().state_vector();
        let delta = source
            .doc
            .transact()
            .encode_state_as_update_v1(&remote_vector);
        assert!(
            remote
                .apply_remote_update_v1(713_024, &delta)
                .unwrap()
                .changed
        );
        assert_cleared(&remote, "accepted remote update");
    }

    #[test]
    fn prepared_active_state_cache_rejection_and_noop_preserve_arc_identity() {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let point = RevisionedPosition {
            offset: 1,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::After,
        };
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 714_000,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: Vec::new(),
                selection_intent: SelectionIntent::Set(SelectionInput::Text {
                    anchor: point,
                    head: point,
                }),
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap();
        engine
            .apply_command(714_001, TypedCommand::InsertText { text: "x".into() })
            .unwrap()
            .unwrap();
        let cache = engine
            .derived_state
            .as_ref()
            .unwrap()
            .active_state_cache_for_test()
            .unwrap();
        let before = atomic_audit(&engine);

        let rejected = engine.apply_typed_transaction(TypedTransaction {
            request_id: 714_002,
            base_document_revision: engine.revision().saturating_add(1),
            origin: TransactionOrigin::LocalApi,
            operations: Vec::new(),
            selection_intent: SelectionIntent::Preserve,
            history_policy: HistoryPolicy::Auto,
        });
        assert!(rejected.is_err());
        assert_eq!(atomic_audit(&engine), before);
        assert!(Arc::ptr_eq(
            &cache,
            &engine
                .derived_state
                .as_ref()
                .unwrap()
                .active_state_cache_for_test()
                .unwrap()
        ));

        let no_op = engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 714_003,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: Vec::new(),
                selection_intent: SelectionIntent::Preserve,
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap();
        assert!(!no_op.changed);
        assert!(Arc::ptr_eq(
            &cache,
            &engine
                .derived_state
                .as_ref()
                .unwrap()
                .active_state_cache_for_test()
                .unwrap()
        ));

        let boundary = engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 714_004,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: Vec::new(),
                selection_intent: SelectionIntent::Preserve,
                history_policy: HistoryPolicy::Boundary,
            })
            .unwrap();
        assert!(!boundary.changed);
        assert!(engine
            .derived_state
            .as_ref()
            .unwrap()
            .active_state_cache_for_test()
            .is_none());
    }

    #[test]
    fn prepared_active_state_warm_hit_matches_forced_generic_at_output_boundaries() {
        use crate::yrs_engine::derived_state::{
            force_active_state_cache_hit_fallback_for_test,
            reset_active_state_cache_counts_for_test, take_active_state_cache_counts_for_test,
        };

        fn fixture(
            json: &str,
            caret: u32,
            first: &str,
            max_derived_output_bytes: usize,
        ) -> YrsDocumentEngine {
            let mut engine = transaction_engine();
            engine.editing_limits.max_derived_output_bytes = max_derived_output_bytes;
            engine
                .import_json(json, TransactionOrigin::DocumentImport)
                .unwrap();
            let point = RevisionedPosition {
                offset: caret,
                kind: EditorOffsetKind::Scalar,
                affinity: Affinity::After,
            };
            engine
                .apply_typed_transaction(TypedTransaction {
                    request_id: 715_000,
                    base_document_revision: engine.revision(),
                    origin: TransactionOrigin::LocalApi,
                    operations: Vec::new(),
                    selection_intent: SelectionIntent::Set(SelectionInput::Text {
                        anchor: point,
                        head: point,
                    }),
                    history_policy: HistoryPolicy::Skip,
                })
                .unwrap();
            engine
                .apply_command(715_001, TypedCommand::InsertText { text: first.into() })
                .unwrap()
                .unwrap();
            assert!(engine
                .derived_state
                .as_ref()
                .unwrap()
                .active_state_cache_for_test()
                .is_some());
            engine
        }

        fn assert_internal_parity(left: &YrsDocumentEngine, right: &YrsDocumentEngine) {
            assert_eq!(left.document_json(), right.document_json());
            assert_eq!(left.can_undo(), right.can_undo());
            assert_eq!(left.can_redo(), right.can_redo());
            let left_state = left.derived_state.as_ref().unwrap();
            let right_state = right.derived_state.as_ref().unwrap();
            assert_eq!(
                left_state.validation_certificate,
                right_state.validation_certificate
            );
            assert_eq!(
                left_state.localized_text_index,
                right_state.localized_text_index
            );
            assert_eq!(
                left_state.render_blocks.materialize(),
                right_state.render_blocks.materialize()
            );
            assert_eq!(
                left_state.active_state_cache_for_test().unwrap().value(),
                right_state.active_state_cache_for_test().unwrap().value()
            );
            for engine in [left, right] {
                let txn = engine.doc.transact();
                let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
                let state = engine.derived_state.as_ref().unwrap();
                assert!(state.mutation_lookup_seed.matches(
                    &txn,
                    &fragment,
                    &state.document,
                    &engine.resource_limits,
                    &engine.editing_limits,
                    engine.max_length,
                    &engine.schema_fingerprint,
                    engine.yrs_state_epoch,
                    engine.revision,
                ));
            }
        }

        for (shape, json, caret, first) in [
            (
                "plain",
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                1,
                "x",
            ),
            (
                "marked",
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc","marks":[{"type":"bold"}]}]}]}"#,
                1,
                "x",
            ),
            (
                "nonBmp",
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"a😀b"}]}]}"#,
                1,
                "🦀",
            ),
        ] {
            // Keep the result-output boundary above the independently enforced
            // deep retained-state budget so the warm certificate exists at
            // both the exact and one-under output limits.
            let second = if shape == "nonBmp" {
                "界".repeat(2_048)
            } else {
                "y".repeat(4_096)
            };
            let mut probe = fixture(json, caret, first, usize::MAX / 2);
            let exact = probe
                .apply_command(
                    715_002,
                    TypedCommand::InsertText {
                        text: second.clone(),
                    },
                )
                .unwrap()
                .unwrap()
                .derived_output_bytes();

            let mut hit = fixture(json, caret, first, exact);
            let mut generic = fixture(json, caret, first, exact);
            reset_active_state_cache_counts_for_test();
            let hit_result = hit
                .apply_command(
                    715_003,
                    TypedCommand::InsertText {
                        text: second.clone(),
                    },
                )
                .unwrap()
                .unwrap();
            assert_eq!(
                take_active_state_cache_counts_for_test(),
                (1, 1, 0, 0, 1, 1, 0, 1, 0),
                "{shape} hit"
            );
            reset_active_state_cache_counts_for_test();
            force_active_state_cache_hit_fallback_for_test(true);
            let generic_result = generic.apply_command(
                715_003,
                TypedCommand::InsertText {
                    text: second.clone(),
                },
            );
            force_active_state_cache_hit_fallback_for_test(false);
            let generic_result = generic_result.unwrap().unwrap();
            assert_eq!(
                take_active_state_cache_counts_for_test(),
                (1, 0, 1, 1, 1, 1, 0, 1, 1),
                "{shape} generic"
            );
            assert_eq!(hit_result.derived_output_bytes(), exact, "{shape}");
            assert_eq!(hit_result, generic_result, "{shape}");
            assert_internal_parity(&hit, &generic);

            let mut rejected_hit = fixture(json, caret, first, exact - 1);
            let mut rejected_generic = fixture(json, caret, first, exact - 1);
            let hit_cache = rejected_hit
                .derived_state
                .as_ref()
                .unwrap()
                .active_state_cache_for_test()
                .unwrap();
            let generic_cache = rejected_generic
                .derived_state
                .as_ref()
                .unwrap()
                .active_state_cache_for_test()
                .unwrap();
            let hit_before = atomic_audit(&rejected_hit);
            let generic_before = atomic_audit(&rejected_generic);
            reset_active_state_cache_counts_for_test();
            let hit_error = rejected_hit
                .apply_command(
                    715_004,
                    TypedCommand::InsertText {
                        text: second.clone(),
                    },
                )
                .unwrap_err();
            assert_eq!(
                take_active_state_cache_counts_for_test(),
                (1, 1, 0, 0, 1, 0, 0, 1, 0),
                "{shape} rejected hit"
            );
            reset_active_state_cache_counts_for_test();
            force_active_state_cache_hit_fallback_for_test(true);
            let generic_error = rejected_generic.apply_command(
                715_004,
                TypedCommand::InsertText {
                    text: second.clone(),
                },
            );
            force_active_state_cache_hit_fallback_for_test(false);
            let generic_error = generic_error.unwrap_err();
            assert_eq!(
                take_active_state_cache_counts_for_test(),
                (1, 0, 1, 1, 1, 0, 0, 1, 1),
                "{shape} rejected generic"
            );
            assert_eq!(hit_error, generic_error, "{shape}");
            assert_eq!(
                hit_error.details,
                Some(json!({ "field": "maxDerivedOutputBytes" })),
                "{shape}"
            );
            assert_eq!(atomic_audit(&rejected_hit), hit_before, "{shape}");
            assert_eq!(atomic_audit(&rejected_generic), generic_before, "{shape}");
            assert!(Arc::ptr_eq(
                &hit_cache,
                &rejected_hit
                    .derived_state
                    .as_ref()
                    .unwrap()
                    .active_state_cache_for_test()
                    .unwrap()
            ));
            assert!(Arc::ptr_eq(
                &generic_cache,
                &rejected_generic
                    .derived_state
                    .as_ref()
                    .unwrap()
                    .active_state_cache_for_test()
                    .unwrap()
            ));
        }
    }

    #[test]
    fn prepared_active_state_context_matrix_matches_forced_generic() {
        use crate::yrs_engine::derived_state::{
            force_active_state_cache_hit_fallback_for_test,
            reset_active_state_cache_counts_for_test, take_active_state_cache_counts_for_test,
        };

        fn fixture(
            shape: &str,
            json: &str,
            target_text: &str,
            intra_leaf_scalar: u32,
            explicit_stored_bold: bool,
        ) -> YrsDocumentEngine {
            let mut engine = transaction_engine();
            engine
                .import_json(json, TransactionOrigin::DocumentImport)
                .unwrap();
            let state = engine.derived_state.as_ref().unwrap();
            let byte_start = state.rendered_text.find(target_text).unwrap();
            let scalar_start =
                u32::try_from(state.rendered_text[..byte_start].chars().count()).unwrap();
            let rendered_position = scalar_start + intra_leaf_scalar;
            let selection_at = |engine: &YrsDocumentEngine, affinity| {
                let point = RevisionedPosition {
                    offset: rendered_position,
                    kind: EditorOffsetKind::Scalar,
                    affinity,
                };
                TypedTransaction {
                    request_id: 716_000,
                    base_document_revision: engine.revision(),
                    origin: TransactionOrigin::LocalApi,
                    operations: Vec::new(),
                    selection_intent: SelectionIntent::Set(SelectionInput::Text {
                        anchor: point,
                        head: point,
                    }),
                    history_policy: HistoryPolicy::Skip,
                }
            };
            if engine
                .apply_typed_transaction(selection_at(&engine, Affinity::After))
                .is_err()
            {
                engine
                    .apply_typed_transaction(selection_at(&engine, Affinity::Before))
                    .unwrap();
            }
            if explicit_stored_bold {
                for request_id in [716_001, 716_002] {
                    engine
                        .apply_command(
                            request_id,
                            TypedCommand::ToggleMark {
                                mark_type: "bold".into(),
                            },
                        )
                        .unwrap()
                        .unwrap();
                }
                assert!(engine
                    .stored_marks()
                    .is_some_and(|marks| { marks.iter().any(|mark| mark.mark_type() == "bold") }));
            }
            engine
                .apply_command(716_003, TypedCommand::InsertText { text: "x".into() })
                .unwrap()
                .unwrap();
            assert!(
                engine
                    .derived_state
                    .as_ref()
                    .unwrap()
                    .active_state_cache_for_test()
                    .is_some(),
                "{shape}"
            );
            engine
        }

        let wide = r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"first"}]},{"type":"paragraph","content":[{"type":"text","text":"middle"}]},{"type":"paragraph","content":[{"type":"text","text":"last"}]}]}"#;
        for (shape, json, target, explicit_stored_bold) in [
            (
                "nested-list-item",
                r#"{"type":"doc","content":[{"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}]}]}"#,
                "abc",
                false,
            ),
            (
                "blockquote",
                r#"{"type":"doc","content":[{"type":"blockquote","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}]}"#,
                "abc",
                false,
            ),
            ("first-top-level", wide, "first", false),
            ("middle-top-level", wide, "middle", false),
            ("last-top-level", wide, "last", false),
            (
                "explicit-stored-marks",
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc","marks":[{"type":"bold"}]}]}]}"#,
                "abc",
                true,
            ),
        ] {
            let mut hit = fixture(shape, json, target, 1, explicit_stored_bold);
            let mut generic = fixture(shape, json, target, 1, explicit_stored_bold);
            reset_active_state_cache_counts_for_test();
            let hit_result = hit
                .apply_command(716_004, TypedCommand::InsertText { text: "y".into() })
                .unwrap()
                .unwrap();
            assert_eq!(
                take_active_state_cache_counts_for_test(),
                (1, 1, 0, 0, 1, 1, 0, 1, 0),
                "{shape} hit"
            );
            reset_active_state_cache_counts_for_test();
            force_active_state_cache_hit_fallback_for_test(true);
            let generic_result =
                generic.apply_command(716_004, TypedCommand::InsertText { text: "y".into() });
            force_active_state_cache_hit_fallback_for_test(false);
            let generic_result = generic_result.unwrap().unwrap();
            assert_eq!(
                take_active_state_cache_counts_for_test(),
                (1, 0, 1, 1, 1, 1, 0, 1, 1),
                "{shape} generic"
            );
            assert_eq!(hit_result, generic_result, "{shape}");
            assert_eq!(hit.document_json(), generic.document_json(), "{shape}");
            assert_eq!(hit.can_undo(), generic.can_undo(), "{shape}");
            assert_eq!(hit.can_redo(), generic.can_redo(), "{shape}");
            let hit_state = hit.derived_state.as_ref().unwrap();
            let generic_state = generic.derived_state.as_ref().unwrap();
            assert_eq!(
                hit_state.validation_certificate, generic_state.validation_certificate,
                "{shape}"
            );
            assert_eq!(
                hit_state.localized_text_index, generic_state.localized_text_index,
                "{shape}"
            );
            assert_eq!(
                hit_state.render_blocks.materialize(),
                generic_state.render_blocks.materialize(),
                "{shape}"
            );
            assert_eq!(
                hit_state.active_state_cache_for_test().unwrap().value(),
                generic_state.active_state_cache_for_test().unwrap().value(),
                "{shape}"
            );
        }
    }

    #[test]
    fn prepared_insert_compilation_uses_localized_semantics_after_planner_step() {
        use crate::yrs_engine::canonical::{
            reset_canonical_artifact_counts_for_test, take_canonical_artifact_counts_for_test,
        };
        use crate::yrs_engine::observability::{
            reset_full_pass_counts_for_test, take_full_pass_counts_for_test,
        };

        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let point = RevisionedPosition {
            offset: 1,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::After,
        };
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 700_137,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: Vec::new(),
                selection_intent: SelectionIntent::Set(SelectionInput::Text {
                    anchor: point,
                    head: point,
                }),
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap();

        engine.ensure_mutation_lookup_seed(700_138).unwrap();
        engine
            .derived_state
            .as_mut()
            .unwrap()
            .materialize_mutation_identity();
        reset_canonical_artifact_counts_for_test();
        let preparation = std::cell::RefCell::new(None);
        let plan = engine
            .plan_command_internal(
                700_138,
                TypedCommand::InsertText { text: "x".into() },
                Some(&preparation),
            )
            .unwrap();
        let CommandPlan::Transaction(transaction) = plan else {
            panic!("insert command must produce a transaction");
        };
        let proof = preparation.into_inner().unwrap();
        assert_eq!(take_canonical_artifact_counts_for_test(), (1, 0));

        reset_full_pass_counts_for_test();
        let compiled = engine
            .compile_prepared_typed_transaction(transaction, proof)
            .unwrap();
        assert!(compiled.localized_insert_admission.is_some());
        assert_eq!(
            take_full_pass_counts_for_test().ordinary_step_applications,
            0
        );
    }

    #[test]
    fn stage4b2_prepared_same_leaf_insert_avoids_postwrite_relative_selection_traversals() {
        use crate::yrs_engine::derived_state::{
            reset_prewrite_selection_proof_counts_for_test,
            reset_relative_selection_traversal_counts_for_test,
            take_prewrite_selection_proof_counts_for_test,
            take_relative_selection_traversal_counts_for_test,
        };

        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let point = RevisionedPosition {
            offset: 1,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::After,
        };
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 700_153,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: Vec::new(),
                selection_intent: SelectionIntent::Set(SelectionInput::Text {
                    anchor: point,
                    head: point,
                }),
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap();

        reset_relative_selection_traversal_counts_for_test();
        reset_prewrite_selection_proof_counts_for_test();
        let result = engine
            .apply_command(700_154, TypedCommand::InsertText { text: "x".into() })
            .unwrap()
            .unwrap();

        assert_eq!(
            result.selection,
            engine.resolved_selection().unwrap().clone()
        );
        assert_eq!(take_relative_selection_traversal_counts_for_test(), (0, 0));
        assert_eq!(
            take_prewrite_selection_proof_counts_for_test(),
            (1, 1, 0, 1)
        );
    }

    #[test]
    fn stage4b2_prepared_selection_tamper_fails_closed_to_generic_parity() {
        use crate::yrs_engine::derived_state::{
            reset_prewrite_selection_proof_counts_for_test,
            reset_relative_selection_traversal_counts_for_test,
            take_prewrite_selection_proof_counts_for_test,
            take_relative_selection_traversal_counts_for_test,
        };

        fn fixture(snapshot: &crate::yrs_engine::DocumentSnapshot) -> YrsDocumentEngine {
            let mut engine = transaction_engine();
            engine.restore_snapshot(snapshot).unwrap();
            let point = RevisionedPosition {
                offset: 1,
                kind: EditorOffsetKind::Scalar,
                affinity: Affinity::After,
            };
            engine
                .apply_typed_transaction(TypedTransaction {
                    request_id: 700_155,
                    base_document_revision: engine.revision(),
                    origin: TransactionOrigin::LocalApi,
                    operations: Vec::new(),
                    selection_intent: SelectionIntent::Set(SelectionInput::Text {
                        anchor: point,
                        head: point,
                    }),
                    history_policy: HistoryPolicy::Skip,
                })
                .unwrap();
            engine
        }

        fn prepared_insert(engine: &YrsDocumentEngine, request_id: u64) -> CompiledTransaction {
            let preparation = std::cell::RefCell::new(None);
            let CommandPlan::Transaction(transaction) = engine
                .plan_command_internal(
                    request_id,
                    TypedCommand::InsertText { text: "x".into() },
                    Some(&preparation),
                )
                .unwrap()
            else {
                panic!("insert command must produce a transaction")
            };
            let proof = preparation.into_inner().unwrap();
            engine
                .compile_prepared_typed_transaction(transaction, proof)
                .unwrap()
        }

        let mut baseline = transaction_engine();
        baseline
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let snapshot = baseline.export_snapshot().unwrap();

        let mut tampered = fixture(&snapshot);
        reset_prewrite_selection_proof_counts_for_test();
        let mut compiled = prepared_insert(&tampered, 700_156);
        compiled.prepared_selection_state = Some(
            compiled
                .prepared_selection_state
                .as_ref()
                .unwrap()
                .tampered_for_test()
                .swap_remove(0),
        );
        reset_relative_selection_traversal_counts_for_test();
        let tampered_result = tampered
            .apply_compiled_transaction(compiled, true)
            .unwrap()
            .1
            .unwrap();
        assert_eq!(take_relative_selection_traversal_counts_for_test(), (1, 1));
        assert_eq!(
            take_prewrite_selection_proof_counts_for_test(),
            (1, 1, 1, 0)
        );

        let mut generic = fixture(&snapshot);
        let mut generic_compiled = prepared_insert(&generic, 700_156);
        generic_compiled.prepared_selection_state = None;
        generic_compiled.prepared_selection_mutation_seal = None;
        reset_relative_selection_traversal_counts_for_test();
        let generic_result = generic
            .apply_compiled_transaction(generic_compiled, true)
            .unwrap()
            .1
            .unwrap();
        assert_eq!(take_relative_selection_traversal_counts_for_test(), (1, 1));
        assert_eq!(tampered_result, generic_result);
        assert_eq!(tampered.document_json(), generic.document_json());
        assert_eq!(tampered.relative_selection(), generic.relative_selection());
        assert_eq!(tampered.resolved_selection(), generic.resolved_selection());
        assert_eq!(tampered.can_undo(), generic.can_undo());

        let mut optimized = fixture(&snapshot);
        let optimized_result = optimized
            .apply_command(700_156, TypedCommand::InsertText { text: "x".into() })
            .unwrap()
            .unwrap();
        assert_eq!(optimized_result, generic_result);
        assert_eq!(optimized.document_json(), generic.document_json());
        assert_eq!(optimized.relative_selection(), generic.relative_selection());
        assert_eq!(optimized.resolved_selection(), generic.resolved_selection());
        assert_eq!(optimized.can_undo(), generic.can_undo());

        assert_eq!(
            tampered.undo(700_157).unwrap(),
            generic.undo(700_157).unwrap()
        );
        optimized.undo(700_157).unwrap();
        assert_eq!(tampered.document_json(), generic.document_json());
        assert_eq!(optimized.document_json(), generic.document_json());
        assert_eq!(
            tampered.redo(700_158).unwrap(),
            generic.redo(700_158).unwrap()
        );
        optimized.redo(700_158).unwrap();
        assert_eq!(tampered.document_json(), generic.document_json());
        assert_eq!(optimized.document_json(), generic.document_json());

        for tamper_index in 0..3 {
            let mut engine = fixture(&snapshot);
            reset_prewrite_selection_proof_counts_for_test();
            let mut compiled = prepared_insert(&engine, 700_160 + tamper_index as u64);
            compiled.prepared_selection_state = Some(
                compiled
                    .prepared_selection_state
                    .as_ref()
                    .unwrap()
                    .tampered_for_test()
                    .swap_remove(tamper_index),
            );
            reset_relative_selection_traversal_counts_for_test();
            engine.apply_compiled_transaction(compiled, true).unwrap();
            assert_eq!(
                take_prewrite_selection_proof_counts_for_test(),
                (1, 1, 1, 0)
            );
            assert_eq!(take_relative_selection_traversal_counts_for_test(), (1, 1));
        }

        let mut engine = fixture(&snapshot);
        let before = atomic_audit(&engine);
        reset_prewrite_selection_proof_counts_for_test();
        let mut compiled = prepared_insert(&engine, 700_163);
        compiled.prepared_selection_mutation_seal = None;
        reset_relative_selection_traversal_counts_for_test();
        let error = engine
            .apply_compiled_transaction(compiled, true)
            .unwrap_err();
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
        assert_eq!(atomic_audit(&engine), before);
        assert_eq!(
            take_prewrite_selection_proof_counts_for_test(),
            (1, 1, 0, 0)
        );
        assert_eq!(take_relative_selection_traversal_counts_for_test(), (0, 0));

        for case in [
            "actionIndex",
            "actionLength",
            "admissionResult",
            "origin",
            "history",
            "selectionPlan",
            "epoch",
            "revision",
        ] {
            let mut engine = fixture(&snapshot);
            let before = atomic_audit(&engine);
            reset_prewrite_selection_proof_counts_for_test();
            let mut compiled = prepared_insert(&engine, 700_164);
            match case {
                "actionIndex" => {
                    let [YrsMutationAction::InsertText { index_utf16, .. }] =
                        compiled.mutation_plan.actions.as_mut_slice()
                    else {
                        unreachable!()
                    };
                    *index_utf16 = index_utf16.saturating_add(1);
                }
                "actionLength" => {
                    let [YrsMutationAction::InsertText { len_utf16, .. }] =
                        compiled.mutation_plan.actions.as_mut_slice()
                    else {
                        unreachable!()
                    };
                    *len_utf16 = len_utf16.saturating_add(1);
                }
                "admissionResult" => {
                    let admission = compiled.localized_insert_admission.as_ref().unwrap();
                    compiled.localized_insert_admission = Some(
                        admission
                            .tampered_claims_for_test()
                            .into_iter()
                            .find(|(claim, _)| *claim == "operationResult")
                            .unwrap()
                            .1,
                    );
                }
                "origin" => compiled.origin = TransactionOrigin::LocalInput,
                "history" => compiled.history_policy = HistoryPolicy::Auto,
                "selectionPlan" => {
                    compiled.selection_plan = SelectionPlan::Explicit(Selection::cursor(1));
                }
                "epoch" => compiled.yrs_state_epoch = compiled.yrs_state_epoch.saturating_add(1),
                "revision" => {
                    compiled.base_state_revision = compiled.base_state_revision.saturating_add(1);
                }
                _ => unreachable!(),
            }
            let authority =
                crate::yrs_engine::prepared_admission::InstalledDerivedStateAuthority::new(
                    engine.derived_state.as_ref().unwrap(),
                );
            assert!(
                !compiled
                    .prepared_selection_mutation_seal
                    .as_ref()
                    .unwrap()
                    .matches(&compiled, &authority),
                "{case}"
            );
            reset_relative_selection_traversal_counts_for_test();
            let error = engine
                .apply_compiled_transaction(compiled, true)
                .unwrap_err();
            assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{case}");
            assert_eq!(atomic_audit(&engine), before, "{case}");
            assert_eq!(
                take_prewrite_selection_proof_counts_for_test(),
                (1, 1, 0, 0),
                "{case}"
            );
            assert_eq!(
                take_relative_selection_traversal_counts_for_test(),
                (0, 0),
                "{case}"
            );
        }

        let mut engine = fixture(&snapshot);
        let before = atomic_audit(&engine);
        reset_prewrite_selection_proof_counts_for_test();
        let mut compiled = prepared_insert(&engine, 700_165);
        let original_target = match compiled.mutation_plan.actions.as_slice() {
            [YrsMutationAction::InsertText { target, .. }] => {
                <XmlTextRef as AsRef<Branch>>::as_ref(target).id()
            }
            _ => unreachable!(),
        };
        let foreign = utf16_doc();
        {
            let update = Update::decode_v1(&snapshot.encoded_state).unwrap();
            foreign.transact_mut().apply_update(update).unwrap();
        }
        let foreign_text = {
            let txn = foreign.transact();
            let fragment = txn.get_xml_fragment("prosemirror").unwrap();
            let XmlOut::Element(paragraph) = fragment.get(&txn, 0).unwrap() else {
                unreachable!()
            };
            let XmlOut::Text(text) = paragraph.get(&txn, 0).unwrap() else {
                unreachable!()
            };
            text
        };
        assert_eq!(
            <XmlTextRef as AsRef<Branch>>::as_ref(&foreign_text).id(),
            original_target
        );
        let [YrsMutationAction::InsertText { target, .. }] =
            compiled.mutation_plan.actions.as_mut_slice()
        else {
            unreachable!()
        };
        *target = foreign_text;
        {
            let authority =
                crate::yrs_engine::prepared_admission::InstalledDerivedStateAuthority::new(
                    engine.derived_state.as_ref().unwrap(),
                );
            assert!(!compiled
                .prepared_selection_mutation_seal
                .as_ref()
                .unwrap()
                .matches(&compiled, &authority));
        }
        let error = engine
            .apply_compiled_transaction(compiled, true)
            .unwrap_err();
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
        assert_eq!(atomic_audit(&engine), before);
        assert_eq!(
            take_prewrite_selection_proof_counts_for_test(),
            (1, 1, 0, 0)
        );
    }

    #[test]
    fn stage4b2_direct_local_insert_does_not_enter_prewrite_selection_proof_lifecycle() {
        use crate::yrs_engine::derived_state::{
            reset_prewrite_selection_proof_counts_for_test,
            reset_relative_selection_traversal_counts_for_test,
            take_prewrite_selection_proof_counts_for_test,
            take_relative_selection_traversal_counts_for_test,
        };

        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        reset_prewrite_selection_proof_counts_for_test();
        reset_relative_selection_traversal_counts_for_test();
        engine
            .apply_typed_transaction_with_result(TypedTransaction {
                request_id: 700_159,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: RevisionedPosition {
                        offset: 1,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    text: "x".into(),
                    marks: Vec::new(),
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            })
            .unwrap();
        assert_eq!(
            take_prewrite_selection_proof_counts_for_test(),
            (0, 0, 0, 0)
        );
        assert_eq!(take_relative_selection_traversal_counts_for_test(), (1, 1));
    }

    #[test]
    fn stage4b2_prepared_failpoints_never_install_selection_proof() {
        use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};
        use crate::yrs_engine::derived_state::{
            reset_prewrite_selection_proof_counts_for_test,
            take_prewrite_selection_proof_counts_for_test,
        };

        for failpoint in [
            AtomicFailpoint::CanonicalOutputAdmission,
            AtomicFailpoint::FinalPreflight,
            AtomicFailpoint::EncodedAdmission,
            AtomicFailpoint::RevisionAdmission,
            AtomicFailpoint::DurableMetadataAdmission,
        ] {
            let mut engine = transaction_engine();
            engine
                .import_json(
                    r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                    TransactionOrigin::DocumentImport,
                )
                .unwrap();
            let point = RevisionedPosition {
                offset: 1,
                kind: EditorOffsetKind::Scalar,
                affinity: Affinity::After,
            };
            engine
                .apply_typed_transaction(TypedTransaction {
                    request_id: 700_166,
                    base_document_revision: engine.revision(),
                    origin: TransactionOrigin::LocalApi,
                    operations: Vec::new(),
                    selection_intent: SelectionIntent::Set(SelectionInput::Text {
                        anchor: point,
                        head: point,
                    }),
                    history_policy: HistoryPolicy::Skip,
                })
                .unwrap();
            hydrate_import_for_compile_test(&mut engine);
            let preparation = std::cell::RefCell::new(None);
            let CommandPlan::Transaction(transaction) = engine
                .plan_command_internal(
                    700_167,
                    TypedCommand::InsertText { text: "x".into() },
                    Some(&preparation),
                )
                .unwrap()
            else {
                unreachable!()
            };
            reset_prewrite_selection_proof_counts_for_test();
            let compiled = engine
                .compile_prepared_typed_transaction(transaction, preparation.into_inner().unwrap())
                .unwrap();
            let before = atomic_audit(&engine);
            set_atomic_failpoint_for_test(Some(failpoint));
            let error = engine
                .apply_compiled_transaction(compiled, true)
                .unwrap_err();
            set_atomic_failpoint_for_test(None);
            assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{failpoint:?}");
            assert_eq!(atomic_audit(&engine), before, "{failpoint:?}");
            assert_eq!(
                take_prewrite_selection_proof_counts_for_test(),
                (1, 1, 0, 0),
                "{failpoint:?}"
            );
        }
    }

    #[test]
    fn stage4b2_optimized_selection_matches_generic_matrix() {
        fn fixture(
            snapshot: &crate::yrs_engine::DocumentSnapshot,
            offset: u32,
        ) -> YrsDocumentEngine {
            let mut engine = transaction_engine();
            engine.restore_snapshot(snapshot).unwrap();
            let point = RevisionedPosition {
                offset,
                kind: EditorOffsetKind::Scalar,
                affinity: Affinity::After,
            };
            engine
                .apply_typed_transaction(TypedTransaction {
                    request_id: 700_170,
                    base_document_revision: engine.revision(),
                    origin: TransactionOrigin::LocalApi,
                    operations: Vec::new(),
                    selection_intent: SelectionIntent::Set(SelectionInput::Text {
                        anchor: point,
                        head: point,
                    }),
                    history_policy: HistoryPolicy::Skip,
                })
                .unwrap();
            engine
        }

        fn prepared_insert(
            engine: &YrsDocumentEngine,
            request_id: u64,
            text: &str,
        ) -> CompiledTransaction {
            let preparation = std::cell::RefCell::new(None);
            let CommandPlan::Transaction(transaction) = engine
                .plan_command_internal(
                    request_id,
                    TypedCommand::InsertText { text: text.into() },
                    Some(&preparation),
                )
                .unwrap()
            else {
                panic!("insert command must produce a transaction")
            };
            let proof = preparation.into_inner().unwrap();
            engine
                .compile_prepared_typed_transaction(transaction, proof)
                .unwrap()
        }

        let cases = [
            (
                "non-bmp",
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                1,
                "🙂",
            ),
            (
                "marked-fragmented",
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","marks":[{"type":"bold"}],"text":"ab"},{"type":"text","marks":[{"type":"italic"}],"text":"cd"}]}]}"#,
                3,
                "x",
            ),
            (
                "nested",
                r#"{"type":"doc","content":[{"type":"blockquote","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}]}"#,
                1,
                "x",
            ),
        ];

        for (index, (case, json, offset, inserted)) in cases.into_iter().enumerate() {
            let request_id = 700_171 + index as u64;
            let mut baseline = transaction_engine();
            baseline
                .import_json(json, TransactionOrigin::DocumentImport)
                .unwrap();
            let snapshot = baseline.export_snapshot().unwrap();
            let mut optimized = fixture(&snapshot, offset);
            let optimized_result = optimized
                .apply_command(
                    request_id,
                    TypedCommand::InsertText {
                        text: inserted.into(),
                    },
                )
                .unwrap()
                .unwrap();

            let mut generic = fixture(&snapshot, offset);
            let mut compiled = prepared_insert(&generic, request_id, inserted);
            assert!(compiled.prepared_selection_state.is_some(), "{case}");
            compiled.prepared_selection_state = None;
            compiled.prepared_selection_mutation_seal = None;
            let generic_result = generic
                .apply_compiled_transaction(compiled, true)
                .unwrap()
                .1
                .unwrap();

            assert_eq!(optimized_result, generic_result, "{case}");
            assert_eq!(optimized.document_json(), generic.document_json(), "{case}");
            assert_eq!(
                optimized.relative_selection(),
                generic.relative_selection(),
                "{case}"
            );
            assert_eq!(
                optimized.resolved_selection(),
                generic.resolved_selection(),
                "{case}"
            );
            assert_eq!(
                optimized.derived_state.as_ref().unwrap().legacy_selection,
                generic.derived_state.as_ref().unwrap().legacy_selection,
                "{case}"
            );
            assert_eq!(optimized.can_undo(), generic.can_undo(), "{case}");
            assert_eq!(
                optimized.undo(700_180).unwrap(),
                generic.undo(700_180).unwrap(),
                "{case}"
            );
            assert_eq!(optimized.document_json(), generic.document_json(), "{case}");
            assert_eq!(
                optimized.redo(700_181).unwrap(),
                generic.redo(700_181).unwrap(),
                "{case}"
            );
            assert_eq!(optimized.document_json(), generic.document_json(), "{case}");
        }
    }

    #[test]
    fn stage4b2_wide_deep_selection_traversal_counts_are_constant() {
        use crate::yrs_engine::derived_state::{
            reset_prewrite_selection_proof_counts_for_test,
            reset_relative_selection_traversal_counts_for_test,
            take_prewrite_selection_proof_counts_for_test,
            take_relative_selection_traversal_counts_for_test,
        };

        let mut observed = Vec::new();
        for factor in [1usize, 2] {
            let mut nested = json!({
                "type": "paragraph",
                "content": [{ "type": "text", "text": "abc" }]
            });
            for _ in 0..(factor * 3) {
                nested = json!({ "type": "blockquote", "content": [nested] });
            }
            let mut content = vec![nested];
            content.extend((1..factor * 32).map(|index| {
                json!({
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": format!("{index:04} abc") }]
                })
            }));
            let mut engine = transaction_engine();
            engine
                .import_json(
                    &json!({ "type": "doc", "content": content }).to_string(),
                    TransactionOrigin::DocumentImport,
                )
                .unwrap();
            let point = RevisionedPosition {
                offset: 1,
                kind: EditorOffsetKind::Scalar,
                affinity: Affinity::After,
            };
            engine
                .apply_typed_transaction(TypedTransaction {
                    request_id: 700_190 + factor as u64,
                    base_document_revision: engine.revision(),
                    origin: TransactionOrigin::LocalApi,
                    operations: Vec::new(),
                    selection_intent: SelectionIntent::Set(SelectionInput::Text {
                        anchor: point,
                        head: point,
                    }),
                    history_policy: HistoryPolicy::Skip,
                })
                .unwrap();
            reset_prewrite_selection_proof_counts_for_test();
            reset_relative_selection_traversal_counts_for_test();
            engine
                .apply_command(
                    700_192 + factor as u64,
                    TypedCommand::InsertText { text: "x".into() },
                )
                .unwrap()
                .unwrap();
            observed.push((
                take_prewrite_selection_proof_counts_for_test(),
                take_relative_selection_traversal_counts_for_test(),
            ));
        }
        assert_eq!(observed[0], observed[1]);
        assert_eq!(observed[0], ((1, 1, 0, 1), (0, 0)));
    }

    #[test]
    fn prepared_command_preserves_semantic_output_error_before_yrs_scan_admission() {
        let mut engine = transaction_engine();
        engine
            .import_json(
                &json!({
                    "type": "doc",
                    "content": [{
                        "type": "paragraph",
                        "content": [{ "type": "text", "text": "x".repeat(4_096) }]
                    }]
                })
                .to_string(),
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        engine.editing_limits.max_derived_output_bytes = 1;
        engine.resource_limits.max_input_bytes = 128;
        let command = TypedCommand::InsertText { text: "y".into() };

        let probe = engine.plan_command(70_005, command.clone()).unwrap_err();
        let exact = usize::try_from(probe.actual.unwrap()).unwrap();
        engine.editing_limits.max_derived_output_bytes = exact;
        assert!(engine.plan_command(70_005, command.clone()).is_ok());
        let before = atomic_audit(&engine);
        let scan_error = engine.apply_command(70_005, command.clone()).unwrap_err();
        assert_eq!(
            scan_error.details,
            Some(json!({ "field": "maxInputBytes" })),
            "{scan_error:?}",
        );
        assert_eq!(atomic_audit(&engine), before);

        engine.editing_limits.max_derived_output_bytes = exact - 1;
        let planned_error = engine.plan_command(70_005, command.clone()).unwrap_err();
        assert_eq!(planned_error.operation_index, Some(0));
        assert_eq!(planned_error.actual, Some(exact as u64));
        assert_eq!(
            planned_error.details,
            Some(json!({ "field": "maxDerivedOutputBytes" }))
        );

        let applied_error = engine.apply_command(70_005, command).unwrap_err();

        assert_eq!(applied_error, planned_error);
        assert_eq!(atomic_audit(&engine), before);
    }

    #[test]
    fn prepared_command_preserves_semantic_undo_error_before_yrs_scan_admission() {
        let mut engine = transaction_engine();
        engine
            .import_json(
                &json!({
                    "type": "doc",
                    "content": [{
                        "type": "paragraph",
                        "content": [{ "type": "text", "text": "x".repeat(4_096) }]
                    }]
                })
                .to_string(),
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        engine.editing_limits.max_undo_retained_units = 0;
        engine.resource_limits.max_input_bytes = 128;
        let command = TypedCommand::InsertText { text: "y".into() };

        let probe = engine.plan_command(70_006, command.clone()).unwrap_err();
        let exact = probe.actual.unwrap();
        engine.editing_limits.max_undo_retained_units = exact;
        assert!(engine.plan_command(70_006, command.clone()).is_ok());
        let before = atomic_audit(&engine);
        let scan_error = engine.apply_command(70_006, command.clone()).unwrap_err();
        assert_eq!(
            scan_error.details,
            Some(json!({ "field": "maxInputBytes" })),
            "{scan_error:?}",
        );
        assert_eq!(atomic_audit(&engine), before);

        engine.editing_limits.max_undo_retained_units = exact - 1;
        let planned_error = engine.plan_command(70_006, command.clone()).unwrap_err();
        assert_eq!(planned_error.operation_index, Some(0));
        assert_eq!(planned_error.actual, Some(exact));
        assert_eq!(
            planned_error.details,
            Some(json!({ "field": "maxUndoRetainedUnits" }))
        );

        let applied_error = engine.apply_command(70_006, command).unwrap_err();

        assert_eq!(applied_error, planned_error);
        assert_eq!(atomic_audit(&engine), before);
    }

    #[test]
    fn prepared_insert_applies_collapsed_stored_marks_in_one_compilation() {
        use crate::yrs_engine::compiler::{
            reset_semantic_compilation_count_for_test, take_semantic_compilation_count_for_test,
        };

        let mut engine = transaction_engine();
        engine
            .apply_command(
                70_010,
                TypedCommand::ToggleMark {
                    mark_type: "bold".into(),
                },
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            engine.stored_marks().unwrap(),
            &[Mark::new("bold".into(), HashMap::new())]
        );
        reset_semantic_compilation_count_for_test();

        engine
            .apply_command(70_011, TypedCommand::InsertText { text: "x".into() })
            .unwrap()
            .unwrap();

        assert_eq!(take_semantic_compilation_count_for_test(), 1);
        assert_eq!(
            engine.document_json().unwrap(),
            json!({
                "type": "doc",
                "content": [{
                    "type": "paragraph",
                    "content": [{
                        "type": "text",
                        "text": "x",
                        "marks": [{ "type": "bold" }]
                    }]
                }]
            })
        );
        assert_eq!(engine.stored_marks(), None);
    }

    #[test]
    fn delete_empty_block_compiles_once_with_exact_selection() {
        use crate::yrs_engine::compiler::{
            reset_semantic_compilation_count_for_test, take_semantic_compilation_count_for_test,
        };

        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"a"}]},{"type":"paragraph"}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let scalar = engine
            .position_map()
            .unwrap()
            .doc_to_scalar(4, engine.document().unwrap());
        let point = RevisionedPosition {
            offset: scalar,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::Before,
        };
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 70_020,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: Vec::new(),
                selection_intent: SelectionIntent::Set(SelectionInput::Text {
                    anchor: point,
                    head: point,
                }),
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap();
        reset_semantic_compilation_count_for_test();

        let result = engine
            .apply_command(70_021, TypedCommand::DeleteBackward)
            .unwrap()
            .unwrap();

        assert_eq!(take_semantic_compilation_count_for_test(), 1);
        assert_eq!(
            engine.document_json().unwrap(),
            json!({
                "type": "doc",
                "content": [{
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": "a" }]
                }]
            })
        );
        let crate::yrs_engine::ResolvedSelection::Text { anchor, head } = result.selection else {
            panic!("structural fallback must preserve a text selection");
        };
        assert_eq!((anchor.scalar, head.scalar), (1, 1));
        assert!(result.history_state.can_undo);
    }

    #[test]
    fn ambiguous_wrap_in_list_keeps_the_public_proof_path() {
        use crate::yrs_engine::compiler::{
            reset_semantic_compilation_count_for_test, take_semantic_compilation_count_for_test,
        };
        use crate::yrs_engine::observability::{
            reset_full_pass_counts_for_test, take_full_pass_counts_for_test,
        };

        let engine = transaction_engine();
        reset_semantic_compilation_count_for_test();
        reset_full_pass_counts_for_test();

        let plan = engine
            .plan_command(
                70_030,
                TypedCommand::WrapInList {
                    list_type: "bulletList".into(),
                    item_type: "listItem".into(),
                },
            )
            .unwrap();

        assert!(matches!(plan, CommandPlan::Transaction(_)));
        assert_eq!(take_semantic_compilation_count_for_test(), 1);
        assert_eq!(take_full_pass_counts_for_test().planner_simulations, 1);
        assert_eq!(
            engine.document_json().unwrap()["content"][0]["type"],
            "paragraph"
        );
    }

    #[test]
    fn prepared_toggle_mark_uses_no_eager_whole_tree_collectors() {
        use crate::yrs_engine::mutation::{
            reset_localized_lookup_counts_for_test, reset_range_format_lowering_counts_for_test,
            take_localized_lookup_counts_for_test, take_range_format_lowering_counts_for_test,
        };

        let mut content = Vec::with_capacity(161);
        content.push(json!({
            "type": "h1",
            "content": [{ "type": "text", "text": "h".repeat(42) }]
        }));
        for index in 0..160 {
            let inline = if index == 0 {
                vec![
                    json!({ "type": "text", "text": "p".repeat(55) }),
                    json!({
                        "type": "text",
                        "text": "b".repeat(55),
                        "marks": [{ "type": "bold" }]
                    }),
                    json!({
                        "type": "text",
                        "text": "i".repeat(55),
                        "marks": [{ "type": "italic" }]
                    }),
                    json!({ "type": "text", "text": "t".repeat(55) }),
                ]
            } else {
                vec![json!({
                    "type": "text",
                    "text": format!("{index:04} {}", "x".repeat(215))
                })]
            };
            content.push(json!({ "type": "paragraph", "content": inline }));
        }
        let mut engine = transaction_engine();
        engine
            .import_json(
                &json!({ "type": "doc", "content": content }).to_string(),
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        select_text(&mut engine, 70_030_000, 44, 52);
        hydrate_import_for_compile_test(&mut engine);

        let before_document = engine.document_json().unwrap();
        let before_selection = engine.resolved_selection().unwrap().clone();
        let mut expected_document = before_document.clone();
        let inline = expected_document["content"][1]["content"]
            .as_array_mut()
            .unwrap();
        inline.splice(
            0..1,
            [
                json!({ "type": "text", "text": "p" }),
                json!({
                    "type": "text",
                    "text": "p".repeat(8),
                    "marks": [{ "type": "bold" }]
                }),
                json!({ "type": "text", "text": "p".repeat(46) }),
            ],
        );

        reset_localized_lookup_counts_for_test();
        reset_range_format_lowering_counts_for_test();
        let result = engine
            .apply_command(
                70_030_001,
                TypedCommand::ToggleMark {
                    mark_type: "bold".into(),
                },
            )
            .unwrap()
            .unwrap();

        assert_eq!(engine.document_json().unwrap(), expected_document);
        assert_eq!(result.selection, before_selection);
        assert_eq!(engine.resolved_selection().unwrap(), &before_selection);
        assert!(result.history_state.can_undo);
        assert!(!result.history_state.can_redo);
        assert!(engine.can_undo());
        assert!(!engine.can_redo());
        let range_format_counts = take_range_format_lowering_counts_for_test();
        let localized_lookup_counts = take_localized_lookup_counts_for_test();
        assert_eq!(localized_lookup_counts, (0, 0, 0));
        assert_eq!(range_format_counts, (0, 0, 1, 0));

        engine.undo(70_030_002).unwrap().unwrap();
        assert_eq!(engine.document_json().unwrap(), before_document);
        assert_eq!(engine.resolved_selection().unwrap(), &before_selection);
        assert!(!engine.can_undo());
        assert!(engine.can_redo());
    }

    #[test]
    fn prepared_reverse_toggle_mark_matches_public_eager_transaction_result() {
        let document = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [
                    { "type": "text", "text": "a😀", "marks": [{ "type": "italic" }] },
                    { "type": "text", "text": "bc" },
                    { "type": "text", "text": "🦀d", "marks": [{ "type": "bold" }] },
                    { "type": "text", "text": "ef" }
                ]
            }]
        });
        let populated = || {
            let mut engine = transaction_engine();
            engine
                .import_json(&document.to_string(), TransactionOrigin::DocumentImport)
                .unwrap();
            select_text(&mut engine, 70_030_100, 7, 1);
            engine
        };
        let command = TypedCommand::ToggleMark {
            mark_type: "bold".into(),
        };

        let mut prepared = populated();
        let prepared_result = prepared
            .apply_command(70_030_101, command.clone())
            .unwrap()
            .unwrap();

        let mut generic = populated();
        let CommandPlan::Transaction(transaction) =
            generic.plan_command(70_030_101, command).unwrap()
        else {
            panic!("reverse toggle-mark must produce a transaction")
        };
        let generic_result = generic
            .apply_typed_transaction_with_result(transaction)
            .unwrap();

        assert_eq!(prepared_result, generic_result);
        assert_eq!(prepared.document_json(), generic.document_json());
        assert_eq!(prepared.document_html(), generic.document_html());
        assert_eq!(prepared.resolved_selection(), generic.resolved_selection());
        assert_eq!(prepared.stored_marks(), generic.stored_marks());
        assert_eq!(prepared.can_undo(), generic.can_undo());
        assert_eq!(prepared.can_redo(), generic.can_redo());
    }

    #[test]
    fn toggle_mark_structural_ranges_reject_before_lowering_with_public_parity() {
        use crate::yrs_engine::mutation::{
            reset_range_format_lowering_counts_for_test, take_range_format_lowering_counts_for_test,
        };

        let cases = [
            (
                "crossBlock",
                json!({
                    "type": "doc",
                    "content": [
                        { "type": "paragraph", "content": [{ "type": "text", "text": "ab" }] },
                        { "type": "paragraph", "content": [{ "type": "text", "text": "cd" }] }
                    ]
                }),
                0,
                5,
                (0, 0, 0, 0),
            ),
            (
                "inlineVoid",
                json!({
                    "type": "doc",
                    "content": [{
                        "type": "paragraph",
                        "content": [
                            { "type": "text", "text": "a" },
                            { "type": "hardBreak" },
                            { "type": "text", "text": "b" }
                        ]
                    }]
                }),
                0,
                3,
                (1, 1, 0, 1),
            ),
        ];

        for (case, document, anchor, head, expected_counts) in cases {
            let populated = || {
                let mut engine = transaction_engine();
                engine
                    .import_json(&document.to_string(), TransactionOrigin::DocumentImport)
                    .unwrap();
                select_text(&mut engine, 70_030_200, anchor, head);
                engine
            };
            let command = TypedCommand::ToggleMark {
                mark_type: "bold".into(),
            };

            let mut prepared = populated();
            let prepared_before = atomic_audit(&prepared);
            reset_range_format_lowering_counts_for_test();
            let prepared_error = prepared
                .apply_command(70_030_201, command.clone())
                .unwrap_err();
            assert_eq!(
                take_range_format_lowering_counts_for_test(),
                expected_counts,
                "{case}"
            );
            assert_eq!(atomic_audit(&prepared), prepared_before, "{case}");

            let mut generic = populated();
            let generic_before = atomic_audit(&generic);
            reset_range_format_lowering_counts_for_test();
            let plan = generic.plan_command(70_030_201, command);
            let generic_error = if case == "crossBlock" {
                let error = plan.unwrap_err();
                assert_eq!(
                    take_range_format_lowering_counts_for_test(),
                    (0, 0, 0, 0),
                    "{case} public plan"
                );
                error
            } else {
                let CommandPlan::Transaction(transaction) = plan.unwrap() else {
                    panic!("{case} must produce a public typed transaction")
                };
                assert_eq!(
                    take_range_format_lowering_counts_for_test(),
                    (0, 0, 0, 0),
                    "{case} public plan"
                );
                reset_range_format_lowering_counts_for_test();
                let error = generic
                    .apply_typed_transaction_with_result(transaction)
                    .unwrap_err();
                assert_eq!(
                    take_range_format_lowering_counts_for_test(),
                    (1, 1, 0, 0),
                    "{case} public apply"
                );
                error
            };
            assert_eq!(prepared_error, generic_error, "{case}");
            assert_eq!(atomic_audit(&generic), generic_before, "{case}");
        }
    }

    #[test]
    fn prepared_toggle_mark_exact_limits_and_one_under_errors_match_public_eager() {
        use crate::yrs_engine::{OperationResult, TypedTransactionResult};

        fn fixture() -> YrsDocumentEngine {
            let mut engine = transaction_engine();
            engine
                .import_json(
                    r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"a😀bc🦀def"}]}]}"#,
                    TransactionOrigin::DocumentImport,
                )
                .unwrap();
            select_text(&mut engine, 70_030_300, 0, 8);
            engine
        }

        fn command() -> TypedCommand {
            TypedCommand::ToggleMark {
                mark_type: "bold".into(),
            }
        }

        fn public_eager_apply(
            engine: &mut YrsDocumentEngine,
            request_id: u64,
        ) -> OperationResult<TypedTransactionResult> {
            let CommandPlan::Transaction(transaction) =
                engine.plan_command(request_id, command())?
            else {
                panic!("range ToggleMark must produce a typed transaction")
            };
            engine.apply_typed_transaction_with_result(transaction)
        }

        fn prepared_apply(
            engine: &mut YrsDocumentEngine,
            request_id: u64,
        ) -> OperationResult<TypedTransactionResult> {
            Ok(engine
                .apply_command(request_id, command())?
                .expect("range ToggleMark must produce a transaction result"))
        }

        fn set_limit(engine: &mut YrsDocumentEngine, field: &str, value: u64) {
            match field {
                "maxUndoRetainedUnits" => {
                    engine.editing_limits.max_undo_retained_units = value;
                }
                "maxInputBytes" => {
                    engine.resource_limits.max_input_bytes = usize::try_from(value).unwrap();
                }
                "maxDerivedOutputBytes" => {
                    engine.editing_limits.max_derived_output_bytes =
                        usize::try_from(value).unwrap();
                }
                "maxEncodedStateBytes" => {
                    engine.resource_limits.max_encoded_state_bytes =
                        usize::try_from(value).unwrap();
                }
                _ => unreachable!(),
            }
        }

        fn exact_limit(field: &str) -> u64 {
            let mut limit = 0;
            loop {
                let mut probe = fixture();
                set_limit(&mut probe, field, limit);
                match public_eager_apply(&mut probe, 70_030_301) {
                    Ok(_) => return limit,
                    Err(error) => {
                        assert_eq!(error.details, Some(json!({ "field": field })), "{field}");
                        let actual = error.actual.expect("limit rejection must report actual");
                        assert!(actual > limit, "{field} probe must make progress");
                        limit = actual;
                    }
                }
            }
        }

        let exact_limits = [
            ("maxUndoRetainedUnits", exact_limit("maxUndoRetainedUnits")),
            ("maxInputBytes", exact_limit("maxInputBytes")),
            (
                "maxDerivedOutputBytes",
                exact_limit("maxDerivedOutputBytes"),
            ),
        ];

        for (index, (field, exact)) in exact_limits.into_iter().enumerate() {
            let request_id = 70_030_310 + u64::try_from(index).unwrap();
            let mut prepared = fixture();
            set_limit(&mut prepared, field, exact);
            let prepared_result = prepared
                .apply_command(request_id, command())
                .unwrap()
                .unwrap();
            let mut generic = fixture();
            set_limit(&mut generic, field, exact);
            let generic_result = public_eager_apply(&mut generic, request_id).unwrap();
            assert_eq!(prepared_result, generic_result, "{field} exact");
            assert_eq!(
                prepared.document_json(),
                generic.document_json(),
                "{field} exact"
            );
            assert_eq!(
                prepared.document_html(),
                generic.document_html(),
                "{field} exact"
            );
            assert_eq!(
                prepared.resolved_selection(),
                generic.resolved_selection(),
                "{field} exact"
            );
            assert_eq!(
                prepared.stored_marks(),
                generic.stored_marks(),
                "{field} exact"
            );
            assert_eq!(prepared.can_undo(), generic.can_undo(), "{field} exact");
            assert_eq!(prepared.can_redo(), generic.can_redo(), "{field} exact");

            let limit = exact
                .checked_sub(1)
                .expect("ToggleMark limits must be nonzero");
            let mut rejected_prepared = fixture();
            set_limit(&mut rejected_prepared, field, limit);
            let prepared_before = atomic_audit(&rejected_prepared);
            let prepared_error = rejected_prepared
                .apply_command(request_id, command())
                .unwrap_err();
            assert_eq!(
                atomic_audit(&rejected_prepared),
                prepared_before,
                "{field} prepared"
            );

            let mut rejected_generic = fixture();
            set_limit(&mut rejected_generic, field, limit);
            let generic_before = atomic_audit(&rejected_generic);
            let generic_error = public_eager_apply(&mut rejected_generic, request_id).unwrap_err();
            assert_eq!(
                atomic_audit(&rejected_generic),
                generic_before,
                "{field} generic"
            );

            assert_eq!(prepared_error, generic_error, "{field}");
            assert_eq!(
                prepared_error.details,
                Some(json!({ "field": field })),
                "{field}"
            );
            assert_eq!(prepared_error.limit, Some(limit), "{field}");
            assert_eq!(prepared_error.actual, Some(exact), "{field}");
        }

        fn exercise_max_encoded_state_boundary(
            request_id: u64,
            apply: fn(&mut YrsDocumentEngine, u64) -> OperationResult<TypedTransactionResult>,
        ) -> (YrsDocumentEngine, TypedTransactionResult) {
            let field = "maxEncodedStateBytes";
            let mut engine = fixture();
            let before = atomic_audit(&engine);
            let current_encoded = u64::try_from(engine.encoded_state().unwrap().len()).unwrap();
            set_limit(&mut engine, field, current_encoded);
            let probe_error = apply(&mut engine, request_id).unwrap_err();
            assert_eq!(atomic_audit(&engine), before, "{field} probe");
            assert_eq!(probe_error.details, Some(json!({ "field": field })));
            let exact = probe_error
                .actual
                .expect("encoded-state rejection must report the exact instance size");
            let one_under = exact
                .checked_sub(1)
                .expect("encoded state must consume at least one byte");

            set_limit(&mut engine, field, one_under);
            let one_under_error = apply(&mut engine, request_id).unwrap_err();
            assert_eq!(atomic_audit(&engine), before, "{field} one-under");
            assert_eq!(one_under_error.details, Some(json!({ "field": field })));
            assert_eq!(one_under_error.limit, Some(one_under));
            assert_eq!(one_under_error.actual, Some(exact));

            set_limit(&mut engine, field, exact);
            let result = apply(&mut engine, request_id).unwrap();
            assert!(engine.encoded_state().unwrap().len() <= usize::try_from(exact).unwrap());
            (engine, result)
        }

        let request_id = 70_030_320;
        let (prepared, prepared_result) =
            exercise_max_encoded_state_boundary(request_id, prepared_apply);
        let (generic, generic_result) =
            exercise_max_encoded_state_boundary(request_id, public_eager_apply);
        assert_eq!(
            prepared_result, generic_result,
            "maxEncodedStateBytes exact"
        );
        assert_eq!(prepared.document_json(), generic.document_json());
        assert_eq!(prepared.document_html(), generic.document_html());
        assert_eq!(prepared.resolved_selection(), generic.resolved_selection());
        assert_eq!(prepared.stored_marks(), generic.stored_marks());
        assert_eq!(prepared.can_undo(), generic.can_undo());
        assert_eq!(prepared.can_redo(), generic.can_redo());
    }

    #[test]
    fn prepared_toggle_and_wrap_commands_each_simulate_and_compile_once() {
        use crate::yrs_engine::compiler::{
            reset_semantic_compilation_count_for_test, take_semantic_compilation_count_for_test,
        };
        use crate::yrs_engine::observability::{
            reset_full_pass_counts_for_test, take_full_pass_counts_for_test,
        };

        let mut toggle = transaction_engine();
        toggle
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        select_text(&mut toggle, 70_031, 0, 2);
        hydrate_import_for_compile_test(&mut toggle);
        reset_semantic_compilation_count_for_test();
        reset_full_pass_counts_for_test();
        toggle
            .apply_command(
                70_032,
                TypedCommand::ToggleMark {
                    mark_type: "bold".into(),
                },
            )
            .unwrap()
            .unwrap();
        assert_eq!(take_semantic_compilation_count_for_test(), 1);
        let toggle_passes = take_full_pass_counts_for_test();
        assert_eq!(toggle_passes.planner_simulations, 1);
        assert_eq!(toggle_passes.document_validations, 1);
        assert_eq!(toggle_passes.canonical_mark_tree_scans, 1);
        assert_eq!(toggle_passes.canonical_projections, 1);
        assert_eq!(toggle_passes.canonical_serializations, 2);
        assert_eq!(toggle_passes.canonical_hashes, 1);
        assert_eq!(toggle_passes.position_map_clones, 0);
        assert_eq!(toggle_passes.position_map_compactions, 0);
        assert_eq!(toggle_passes.rendered_text_derivations, 1);

        let mut wrap = transaction_engine();
        hydrate_import_for_compile_test(&mut wrap);
        reset_semantic_compilation_count_for_test();
        reset_full_pass_counts_for_test();
        wrap.apply_command(
            70_033,
            TypedCommand::WrapInList {
                list_type: "bulletList".into(),
                item_type: "listItem".into(),
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(take_semantic_compilation_count_for_test(), 1);
        let wrap_passes = take_full_pass_counts_for_test();
        assert_eq!(wrap_passes.planner_simulations, 1);
        assert_eq!(wrap_passes.document_validations, 1);
        assert_eq!(wrap_passes.canonical_mark_tree_scans, 1);
        assert_eq!(wrap_passes.canonical_projections, 1);
        assert_eq!(wrap_passes.canonical_serializations, 2);
        assert_eq!(wrap_passes.canonical_hashes, 1);
        assert_eq!(wrap_passes.position_map_clones, 0);
        assert_eq!(wrap_passes.position_map_compactions, 0);
        assert_eq!(wrap_passes.rendered_text_derivations, 1);
        assert_eq!(
            wrap.document_json().unwrap()["content"][0]["type"],
            "bulletList"
        );
    }

    #[test]
    fn prepared_wrap_at_a_block_boundary_matches_its_simulated_selection() {
        let document = json!({
            "type": "doc",
            "content": [
                {
                    "type": "h1",
                    "content": [{ "type": "text", "text": "x".repeat(42) }]
                },
                {
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": "y".repeat(220) }]
                }
            ]
        });
        let populated = || {
            let mut engine = transaction_engine();
            engine
                .import_json(&document.to_string(), TransactionOrigin::DocumentImport)
                .unwrap();
            select_text(&mut engine, 70_033_001, 44, 44);
            engine
        };

        let mut prepared = populated();
        crate::yrs_engine::compiler::reset_semantic_compilation_count_for_test();
        let prepared_result = prepared
            .apply_command(
                70_033_002,
                TypedCommand::WrapInList {
                    list_type: "bulletList".into(),
                    item_type: "listItem".into(),
                },
            )
            .unwrap();
        assert_eq!(
            crate::yrs_engine::compiler::take_semantic_compilation_count_for_test(),
            1
        );

        let mut generic = populated();
        let CommandPlan::Transaction(transaction) = generic
            .plan_command(
                70_033_002,
                TypedCommand::WrapInList {
                    list_type: "bulletList".into(),
                    item_type: "listItem".into(),
                },
            )
            .unwrap()
        else {
            panic!("public block-boundary wrap must produce a transaction")
        };
        let generic_result = generic
            .apply_typed_transaction_with_result(transaction)
            .unwrap();

        assert_eq!(prepared.document_json(), generic.document_json());
        assert_eq!(prepared.resolved_selection(), generic.resolved_selection());
        assert_eq!(prepared_result.unwrap().selection, generic_result.selection);
    }

    #[test]
    fn prepared_article_wrap_uses_only_the_localized_root_window() {
        use crate::yrs_engine::mutation::{
            reset_root_window_lowering_counts_for_test, take_root_window_lowering_counts_for_test,
        };

        let mut content = Vec::with_capacity(161);
        content.push(json!({
            "type": "h1",
            "content": [{ "type": "text", "text": "h".repeat(42) }]
        }));
        for index in 0..160 {
            content.push(json!({
                "type": "paragraph",
                "content": [{
                    "type": "text",
                    "text": format!("{index:04} {}", "x".repeat(215))
                }]
            }));
        }
        let document = json!({ "type": "doc", "content": content });
        let mut engine = transaction_engine();
        engine
            .import_json(&document.to_string(), TransactionOrigin::DocumentImport)
            .unwrap();
        select_text(&mut engine, 70_033_100, 44, 44);
        hydrate_import_for_compile_test(&mut engine);

        let before_document = engine.document_json().unwrap();
        let before_selection = engine.resolved_selection().unwrap().clone();
        let before_revision = engine.revision();
        let mut expected_document = before_document.clone();
        let root_content = expected_document["content"].as_array_mut().unwrap();
        let paragraph = root_content.remove(1);
        root_content.insert(
            1,
            json!({
                "type": "bulletList",
                "content": [{
                    "type": "listItem",
                    "content": [paragraph]
                }]
            }),
        );

        reset_root_window_lowering_counts_for_test();
        let result = engine
            .apply_command(
                70_033_101,
                TypedCommand::WrapInList {
                    list_type: "bulletList".into(),
                    item_type: "listItem".into(),
                },
            )
            .unwrap()
            .unwrap();
        let observed_counts = take_root_window_lowering_counts_for_test();

        assert_eq!(result.request_id, 70_033_101);
        assert_eq!(result.origin, TransactionOrigin::LocalCommand);
        assert!(result.changed);
        assert_eq!(result.document_revision, before_revision + 1);
        assert_eq!(engine.document_json().unwrap(), expected_document);
        assert!(matches!(
            result.selection,
            ResolvedSelection::Text { ref anchor, ref head }
                if (anchor.scalar, head.scalar) == (46, 46)
        ));
        assert_eq!(engine.resolved_selection().unwrap(), &result.selection);
        assert!(result.history_state.can_undo);
        assert!(!result.history_state.can_redo);
        assert!(engine.can_undo());
        assert!(!engine.can_redo());

        reset_root_window_lowering_counts_for_test();
        engine.undo(70_033_102).unwrap().unwrap();
        assert_eq!(engine.document_json().unwrap(), before_document);
        assert_eq!(engine.resolved_selection().unwrap(), &before_selection);
        assert!(!engine.can_undo());
        assert!(engine.can_redo());

        let redo = engine.redo_with_result(70_033_103).unwrap().unwrap();
        assert_eq!(redo.request_id, 70_033_103);
        assert_eq!(redo.origin, TransactionOrigin::UndoRedo);
        assert!(redo.changed);
        assert_eq!(redo.document_revision, before_revision + 3);
        assert_eq!(engine.document_json().unwrap(), expected_document);
        assert!(matches!(
            redo.selection,
            ResolvedSelection::Text { ref anchor, ref head }
                if (anchor.scalar, head.scalar) == (46, 46)
        ));
        assert_eq!(engine.resolved_selection().unwrap(), &redo.selection);
        assert!(redo.history_state.can_undo);
        assert!(!redo.history_state.can_redo);
        assert!(engine.can_undo());
        assert!(!engine.can_redo());

        assert_eq!(observed_counts, (0, 0, 1, 0, 0, 1));
    }

    #[test]
    fn prepared_wrap_proof_binds_the_exact_transaction_and_candidate_identity() {
        let compile = |engine: &YrsDocumentEngine, request_id| {
            let preparation = std::cell::RefCell::new(None);
            let CommandPlan::Transaction(transaction) = engine
                .plan_command_internal(
                    request_id,
                    TypedCommand::WrapInList {
                        list_type: "bulletList".into(),
                        item_type: "listItem".into(),
                    },
                    Some(&preparation),
                )
                .unwrap()
            else {
                panic!("wrap command must produce a transaction")
            };
            (transaction, preparation.into_inner().unwrap())
        };

        let engine = transaction_engine();
        let before = atomic_audit(&engine);
        let (mut transaction, proof) = compile(&engine, 70_034);
        assert!(matches!(
            transaction.operations.as_slice(),
            [TypedOperation::ReplaceStructure(_)]
        ));
        transaction.selection_intent = SelectionIntent::Preserve;
        let error = engine
            .compile_prepared_typed_transaction(transaction, proof)
            .unwrap_err();
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
        assert_eq!(atomic_audit(&engine), before);

        let (transaction, mut proof) = compile(&engine, 70_035);
        proof.document = engine.document().unwrap().clone();
        let error = engine
            .compile_prepared_typed_transaction(transaction, proof)
            .unwrap_err();
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
        assert_eq!(atomic_audit(&engine), before);

        let (transaction, mut proof) = compile(&engine, 70_035_000);
        let base_artifact = engine
            .derived_state
            .as_ref()
            .unwrap()
            .canonical_artifact
            .clone();
        proof
            .eager_semantic_admission_mut_for_test()
            .replace_candidate_artifact_for_test(base_artifact);
        let error = engine
            .compile_prepared_typed_transaction(transaction, proof)
            .unwrap_err();
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
        assert_eq!(atomic_audit(&engine), before);
    }

    #[test]
    fn prepared_wrap_proof_rejects_resource_limit_context_drift() {
        let mut engine = transaction_engine();
        let preparation = std::cell::RefCell::new(None);
        let CommandPlan::Transaction(transaction) = engine
            .plan_command_internal(
                70_035_001,
                TypedCommand::WrapInList {
                    list_type: "bulletList".into(),
                    item_type: "listItem".into(),
                },
                Some(&preparation),
            )
            .unwrap()
        else {
            panic!("prepared wrap must produce a transaction")
        };
        let proof = preparation.into_inner().unwrap();
        engine.resource_limits.max_schema_nodes -= 1;
        let before = atomic_audit(&engine);

        let error = engine
            .compile_prepared_typed_transaction(transaction, proof)
            .unwrap_err();

        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
        assert_eq!(atomic_audit(&engine), before);
    }

    #[test]
    fn prepared_insert_without_candidate_certificate_runs_live_preview_validation() {
        use crate::yrs_engine::compiler::force_localized_semantic_allocation_failure_for_test;
        use crate::yrs_engine::observability::{
            reset_full_pass_counts_for_test, take_full_pass_counts_for_test,
        };

        let engine = transaction_engine();
        let preparation = std::cell::RefCell::new(None);
        let CommandPlan::Transaction(transaction) = engine
            .plan_command_internal(
                70_035_010,
                TypedCommand::InsertText { text: "x".into() },
                Some(&preparation),
            )
            .unwrap()
        else {
            panic!("prepared insert must produce a transaction")
        };
        let proof = preparation.into_inner().unwrap();

        reset_full_pass_counts_for_test();
        force_localized_semantic_allocation_failure_for_test(true);
        let compiled = engine.compile_prepared_typed_transaction(transaction, proof);
        force_localized_semantic_allocation_failure_for_test(false);

        compiled.unwrap();
        let counts = take_full_pass_counts_for_test();
        assert!(counts.document_validations >= 1);
        assert!(counts.canonical_mark_tree_scans >= 1);
    }

    #[test]
    fn prepared_insert_rejects_stale_root_and_foreign_canonical_context_artifacts() {
        let compile = |engine: &YrsDocumentEngine, request_id| {
            let preparation = std::cell::RefCell::new(None);
            let CommandPlan::Transaction(transaction) = engine
                .plan_command_internal(
                    request_id,
                    TypedCommand::InsertText { text: "x".into() },
                    Some(&preparation),
                )
                .unwrap()
            else {
                panic!("prepared insert must produce a transaction")
            };
            (transaction, preparation.into_inner().unwrap())
        };

        let engine = transaction_engine();
        let separate = transaction_engine();
        let before = atomic_audit(&engine);

        let (transaction, mut proof) = compile(&engine, 70_035_011);
        let stale_root_artifact = engine
            .canonical_schema
            .derive(separate.document().unwrap())
            .unwrap();
        proof
            .eager_semantic_admission_mut_for_test()
            .replace_canonical_artifact_for_test(stale_root_artifact);
        let error = engine
            .compile_prepared_typed_transaction(transaction, proof)
            .unwrap_err();
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
        assert_eq!(atomic_audit(&engine), before);

        let (transaction, mut proof) = compile(&engine, 70_035_012);
        let foreign_context_artifact = separate.canonical_schema.derive(&proof.document).unwrap();
        proof
            .eager_semantic_admission_mut_for_test()
            .replace_canonical_artifact_for_test(foreign_context_artifact);
        let error = engine
            .compile_prepared_typed_transaction(transaction, proof)
            .unwrap_err();
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
        assert_eq!(atomic_audit(&engine), before);
    }

    #[test]
    fn prepared_candidate_rejects_foreign_same_total_position_layout() {
        let engine = transaction_engine();
        let preparation = std::cell::RefCell::new(None);
        let CommandPlan::Transaction(transaction) = engine
            .plan_command_internal(
                70_035_012_001,
                TypedCommand::WrapInList {
                    list_type: "bulletList".into(),
                    item_type: "listItem".into(),
                },
                Some(&preparation),
            )
            .unwrap()
        else {
            panic!("prepared wrap must produce a transaction")
        };
        let proof = preparation.into_inner().unwrap();

        let mut foreign = transaction_engine();
        foreign
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let foreign_map =
            crate::position::PositionMap::build(foreign.document().unwrap(), &engine.schema);
        let expected_map = crate::position::PositionMap::build(&proof.document, &engine.schema);
        assert_eq!(foreign_map.total_scalars(), expected_map.total_scalars());
        assert_ne!(
            foreign_map.block(0).unwrap().node_path,
            expected_map.block(0).unwrap().node_path
        );
        let foreign_seed = crate::yrs_engine::compiler::PreparedCandidateSeed::mint(
            transaction.request_id,
            foreign.document().unwrap(),
            &engine.schema,
            &engine.canonical_schema,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
        )
        .unwrap();

        let error =
            crate::yrs_engine::compiler::PreparedSemanticAdmission::prepare_single_operation(
                transaction.request_id,
                engine.revision,
                engine.state_revision,
                engine.yrs_state_epoch,
                &engine.schema,
                &engine.canonical_schema,
                &engine.resource_limits,
                &engine.editing_limits,
                engine.max_length,
                &transaction,
                &proof.document,
                Some(foreign_seed),
                None,
                0,
                crate::yrs_engine::compiler::PreparedCommandContractKind::None,
            )
            .unwrap_err();

        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    }

    #[test]
    fn prepared_wrap_rejects_max_length_context_drift_atomically() {
        let mut engine = transaction_engine();
        let preparation = std::cell::RefCell::new(None);
        let CommandPlan::Transaction(transaction) = engine
            .plan_command_internal(
                70_035_013,
                TypedCommand::WrapInList {
                    list_type: "bulletList".into(),
                    item_type: "listItem".into(),
                },
                Some(&preparation),
            )
            .unwrap()
        else {
            panic!("prepared wrap must produce a transaction")
        };
        let proof = preparation.into_inner().unwrap();
        engine.max_length = Some(0);
        let before = atomic_audit(&engine);

        let error = engine
            .compile_prepared_typed_transaction(transaction, proof)
            .unwrap_err();

        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
        assert_eq!(atomic_audit(&engine), before);
    }

    #[test]
    fn prepared_wrap_rejects_editing_limit_context_drift_atomically() {
        let mut engine = transaction_engine();
        let preparation = std::cell::RefCell::new(None);
        let CommandPlan::Transaction(transaction) = engine
            .plan_command_internal(
                70_035_014,
                TypedCommand::WrapInList {
                    list_type: "bulletList".into(),
                    item_type: "listItem".into(),
                },
                Some(&preparation),
            )
            .unwrap()
        else {
            panic!("prepared wrap must produce a transaction")
        };
        let proof = preparation.into_inner().unwrap();
        engine.editing_limits.max_undo_groups -= 1;
        let before = atomic_audit(&engine);

        let error = engine
            .compile_prepared_typed_transaction(transaction, proof)
            .unwrap_err();

        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
        assert_eq!(atomic_audit(&engine), before);
    }

    #[test]
    fn prepared_wrap_hard_limit_rejection_is_atomic() {
        use crate::yrs_engine::mutation::{
            reset_root_window_lowering_counts_for_test, take_root_window_lowering_counts_for_test,
        };

        let mut engine = transaction_engine();
        engine.resource_limits.max_input_bytes = 0;
        let before = atomic_audit(&engine);

        reset_root_window_lowering_counts_for_test();
        let error = engine
            .apply_command(
                70_036,
                TypedCommand::WrapInList {
                    list_type: "bulletList".into(),
                    item_type: "listItem".into(),
                },
            )
            .unwrap_err();
        let counts = take_root_window_lowering_counts_for_test();

        assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED");
        assert_eq!(error.details, Some(json!({ "field": "maxInputBytes" })));
        assert_eq!((counts.2, counts.3), (0, 0));
        assert_eq!(atomic_audit(&engine), before);
    }

    #[test]
    fn prepared_wrap_accepts_exact_output_limit_and_rejects_one_over_atomically() {
        use crate::yrs_engine::mutation::{
            reset_root_window_lowering_counts_for_test, take_root_window_lowering_counts_for_test,
        };

        let command = TypedCommand::WrapInList {
            list_type: "bulletList".into(),
            item_type: "listItem".into(),
        };
        let mut exact = 1;
        loop {
            let mut probe = transaction_engine();
            probe.editing_limits.max_derived_output_bytes = exact;
            match probe.apply_command(70_036_001, command.clone()) {
                Ok(Some(_)) => break,
                Err(error)
                    if error.details == Some(json!({ "field": "maxDerivedOutputBytes" })) =>
                {
                    let required = usize::try_from(error.actual.unwrap()).unwrap();
                    assert!(required > exact);
                    exact = required;
                }
                outcome => panic!("unexpected output-limit probe result: {outcome:?}"),
            }
        }

        let mut exact_limit = transaction_engine();
        exact_limit.editing_limits.max_derived_output_bytes = exact;
        reset_root_window_lowering_counts_for_test();
        assert!(exact_limit
            .apply_command(70_036_002, command.clone())
            .unwrap()
            .is_some());
        let exact_counts = take_root_window_lowering_counts_for_test();
        assert_eq!((exact_counts.2, exact_counts.3), (1, 0));

        let mut one_over = transaction_engine();
        one_over.editing_limits.max_derived_output_bytes = exact - 1;
        let before = atomic_audit(&one_over);
        reset_root_window_lowering_counts_for_test();
        let error = one_over.apply_command(70_036_003, command).unwrap_err();
        let rejected_counts = take_root_window_lowering_counts_for_test();
        assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
        assert_eq!(error.actual, Some(exact as u64));
        assert_eq!(
            error.details,
            Some(json!({ "field": "maxDerivedOutputBytes" }))
        );
        assert_eq!((rejected_counts.2, rejected_counts.3), (1, 0));
        assert_eq!(atomic_audit(&one_over), before);
    }

    #[test]
    fn prepared_wrap_undo_and_encoded_limits_match_public_eager_exactly() {
        use crate::yrs_engine::mutation::{
            reset_root_window_lowering_counts_for_test, take_root_window_lowering_counts_for_test,
        };
        use crate::yrs_engine::{EditingLimits, OperationResult, TypedTransactionResult};

        fn command() -> TypedCommand {
            TypedCommand::WrapInList {
                list_type: "bulletList".into(),
                item_type: "listItem".into(),
            }
        }

        fn fixture(field: &str, value: u64) -> YrsDocumentEngine {
            let mut resource_limits = ResourceLimits::default();
            let mut editing_limits = EditingLimits::default();
            match field {
                "maxUndoRetainedUnits" => editing_limits.max_undo_retained_units = value,
                "maxEncodedStateBytes" => {
                    resource_limits.max_encoded_state_bytes = usize::try_from(value).unwrap()
                }
                _ => unreachable!(),
            }
            YrsDocumentEngine::new(YrsEngineConfig {
                schema: tiptap_schema(),
                fragment_name: "prosemirror".into(),
                initialization_mode: crate::yrs_engine::InitializationMode::LocalEmpty,
                resource_limits,
                editing_limits,
                max_length: None,
                scope: Some(crate::yrs_engine::DocumentScope {
                    document_id: "doc".into(),
                    lineage_id: "lineage".into(),
                }),
            })
            .unwrap()
        }

        fn public_eager_apply(
            engine: &mut YrsDocumentEngine,
            request_id: u64,
        ) -> OperationResult<TypedTransactionResult> {
            let CommandPlan::Transaction(transaction) =
                engine.plan_command(request_id, command())?
            else {
                panic!("WrapInList must produce a public typed transaction")
            };
            engine.apply_typed_transaction_with_result(transaction)
        }

        fn prepared_apply(
            engine: &mut YrsDocumentEngine,
            request_id: u64,
        ) -> OperationResult<TypedTransactionResult> {
            Ok(engine
                .apply_command(request_id, command())?
                .expect("WrapInList must produce a transaction result"))
        }

        fn exact_undo_limit() -> u64 {
            let field = "maxUndoRetainedUnits";
            let mut limit = 1;
            loop {
                let mut probe = fixture(field, limit);
                match public_eager_apply(&mut probe, 70_036_010) {
                    Ok(_) => return limit,
                    Err(error) => {
                        assert_eq!(error.details, Some(json!({ "field": field })), "{field}");
                        let actual = error.actual.expect("limit rejection must report actual");
                        assert!(actual > limit, "{field} probe must make progress");
                        limit = actual;
                    }
                }
            }
        }

        let field = "maxUndoRetainedUnits";
        let exact = exact_undo_limit();
        let request_id = 70_036_020;

        let mut prepared = fixture(field, exact);
        reset_root_window_lowering_counts_for_test();
        let prepared_result = prepared
            .apply_command(request_id, command())
            .unwrap()
            .unwrap();
        assert_eq!(
            take_root_window_lowering_counts_for_test(),
            (0, 0, 1, 0, 0, 1),
            "{field} prepared exact"
        );

        let mut generic = fixture(field, exact);
        reset_root_window_lowering_counts_for_test();
        let generic_result = public_eager_apply(&mut generic, request_id).unwrap();
        assert_eq!(
            take_root_window_lowering_counts_for_test(),
            (1, 1, 0, 0, 1, 0),
            "{field} eager exact"
        );
        assert_eq!(prepared_result, generic_result, "{field} exact");
        assert_eq!(prepared.document_json(), generic.document_json(), "{field}");
        assert_eq!(prepared.document_html(), generic.document_html(), "{field}");
        assert_eq!(
            prepared.resolved_selection(),
            generic.resolved_selection(),
            "{field}"
        );
        assert_eq!(prepared.stored_marks(), generic.stored_marks(), "{field}");
        assert_eq!(prepared.can_undo(), generic.can_undo(), "{field}");
        assert_eq!(prepared.can_redo(), generic.can_redo(), "{field}");

        let limit = exact.checked_sub(1).expect("wrap limits must be nonzero");
        let mut rejected_prepared = fixture(field, limit);
        let prepared_before = atomic_audit(&rejected_prepared);
        reset_root_window_lowering_counts_for_test();
        let prepared_error = rejected_prepared
            .apply_command(request_id, command())
            .unwrap_err();
        let prepared_counts = take_root_window_lowering_counts_for_test();
        assert_eq!(atomic_audit(&rejected_prepared), prepared_before, "{field}");

        let mut rejected_generic = fixture(field, limit);
        let generic_before = atomic_audit(&rejected_generic);
        reset_root_window_lowering_counts_for_test();
        let generic_error = public_eager_apply(&mut rejected_generic, request_id).unwrap_err();
        let generic_counts = take_root_window_lowering_counts_for_test();
        assert_eq!(atomic_audit(&rejected_generic), generic_before, "{field}");
        assert_eq!(prepared_error, generic_error, "{field}");
        assert_eq!(
            prepared_error.details,
            Some(json!({ "field": field })),
            "{field}"
        );
        assert_eq!(prepared_error.limit, Some(limit), "{field}");
        assert_eq!(prepared_error.actual, Some(exact), "{field}");

        let expected_prepared_counts = (0, 0, 1, 0, 0, 0);
        let expected_generic_counts = (1, 1, 0, 0, 0, 0);
        assert_eq!(
            prepared_counts, expected_prepared_counts,
            "{field} prepared reject"
        );
        assert_eq!(
            generic_counts, expected_generic_counts,
            "{field} eager reject"
        );

        fn exercise_max_encoded_state_boundary(
            request_id: u64,
            apply: fn(&mut YrsDocumentEngine, u64) -> OperationResult<TypedTransactionResult>,
            probe_counts: (usize, usize, usize, usize, usize, usize),
            rejected_counts: (usize, usize, usize, usize, usize, usize),
            success_counts: (usize, usize, usize, usize, usize, usize),
        ) -> (YrsDocumentEngine, TypedTransactionResult) {
            let field = "maxEncodedStateBytes";
            let default_limit =
                u64::try_from(ResourceLimits::default().max_encoded_state_bytes).unwrap();
            let mut engine = fixture(field, default_limit);
            let before = atomic_audit(&engine);
            let current_encoded = u64::try_from(engine.encoded_state().unwrap().len()).unwrap();

            engine.resource_limits.max_encoded_state_bytes =
                usize::try_from(current_encoded).unwrap();
            reset_root_window_lowering_counts_for_test();
            let probe_error = apply(&mut engine, request_id).unwrap_err();
            assert_eq!(
                take_root_window_lowering_counts_for_test(),
                probe_counts,
                "{field} probe"
            );
            assert_eq!(atomic_audit(&engine), before, "{field} probe");
            assert_eq!(probe_error.code, "DOCUMENT_LIMIT_EXCEEDED");
            assert_eq!(probe_error.details, Some(json!({ "field": field })));
            assert_eq!(probe_error.limit, Some(current_encoded));
            let exact = probe_error
                .actual
                .expect("encoded-state rejection must report the exact instance size");
            assert!(exact > current_encoded);
            let one_under = exact
                .checked_sub(1)
                .expect("encoded state must consume at least one byte");

            engine.resource_limits.max_encoded_state_bytes = usize::try_from(one_under).unwrap();
            reset_root_window_lowering_counts_for_test();
            let one_under_error = apply(&mut engine, request_id).unwrap_err();
            assert_eq!(
                take_root_window_lowering_counts_for_test(),
                rejected_counts,
                "{field} one-under"
            );
            assert_eq!(atomic_audit(&engine), before, "{field} one-under");
            assert_eq!(one_under_error.code, "DOCUMENT_LIMIT_EXCEEDED");
            assert_eq!(one_under_error.details, Some(json!({ "field": field })));
            assert_eq!(one_under_error.limit, Some(one_under));
            assert_eq!(one_under_error.actual, Some(exact));

            engine.resource_limits.max_encoded_state_bytes = usize::try_from(exact).unwrap();
            reset_root_window_lowering_counts_for_test();
            let result = apply(&mut engine, request_id).unwrap();
            assert_eq!(
                take_root_window_lowering_counts_for_test(),
                success_counts,
                "{field} exact"
            );
            assert!(engine.encoded_state().unwrap().len() <= usize::try_from(exact).unwrap());
            (engine, result)
        }

        let request_id = 70_036_021;
        // The mutation entry point refreshes the ResourceLimits-bound lookup
        // seed before compilation, so the prepared root window remains valid.
        let (prepared, prepared_result) = exercise_max_encoded_state_boundary(
            request_id,
            prepared_apply,
            (0, 0, 1, 0, 1, 0),
            (0, 0, 1, 0, 1, 0),
            (0, 0, 1, 0, 1, 1),
        );
        let (generic, generic_result) = exercise_max_encoded_state_boundary(
            request_id,
            public_eager_apply,
            (1, 1, 0, 0, 1, 0),
            (1, 1, 0, 0, 1, 0),
            (1, 1, 0, 0, 2, 0),
        );
        assert_eq!(
            prepared_result, generic_result,
            "maxEncodedStateBytes exact"
        );
        assert_eq!(prepared.document_json(), generic.document_json());
        assert_eq!(prepared.document_html(), generic.document_html());
        assert_eq!(prepared.resolved_selection(), generic.resolved_selection());
        assert_eq!(prepared.stored_marks(), generic.stored_marks());
        assert_eq!(prepared.can_undo(), generic.can_undo());
        assert_eq!(prepared.can_redo(), generic.can_redo());
    }

    #[test]
    fn prepared_wrap_is_atomic_at_every_recoverable_failpoint() {
        use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};
        use crate::yrs_engine::mutation::{
            reset_root_window_lowering_counts_for_test, take_root_window_lowering_counts_for_test,
        };

        let failpoints = [
            AtomicFailpoint::EnvelopeAdmission,
            AtomicFailpoint::SemanticCompilation,
            AtomicFailpoint::MutationPreflight,
            AtomicFailpoint::FinalPreflight,
            AtomicFailpoint::EncodedAdmission,
            AtomicFailpoint::CanonicalOutputAdmission,
            AtomicFailpoint::RevisionAdmission,
            AtomicFailpoint::DurableMetadataAdmission,
        ];
        for (index, failpoint) in failpoints.into_iter().enumerate() {
            let mut engine = transaction_engine();
            let before = atomic_audit(&engine);
            let seed_before = engine
                .derived_state
                .as_ref()
                .unwrap()
                .mutation_lookup_seed
                .clone();
            assert!(seed_before.is_ready_for_test());
            reset_root_window_lowering_counts_for_test();
            set_atomic_failpoint_for_test(Some(failpoint));

            let error = engine
                .apply_command(
                    70_036_100 + index as u64,
                    TypedCommand::WrapInList {
                        list_type: "bulletList".into(),
                        item_type: "listItem".into(),
                    },
                )
                .unwrap_err();

            set_atomic_failpoint_for_test(None);
            assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{failpoint:?}");
            assert_eq!(
                error.details,
                Some(json!({ "failpoint": failpoint.field_name() })),
                "{failpoint:?}"
            );
            assert_eq!(
                take_root_window_lowering_counts_for_test().5,
                0,
                "{failpoint:?}"
            );
            assert!(Arc::ptr_eq(
                &seed_before,
                &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
            ));
            assert!(engine
                .derived_state
                .as_ref()
                .unwrap()
                .mutation_lookup_seed
                .is_ready_for_test());
            assert_eq!(atomic_audit(&engine), before, "{failpoint:?}");
        }
    }

    #[test]
    fn prepared_toggle_mark_is_atomic_at_every_recoverable_failpoint() {
        use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};
        use crate::yrs_engine::mutation::{
            reset_localized_lookup_counts_for_test, reset_range_format_lowering_counts_for_test,
            take_localized_lookup_counts_for_test, take_range_format_lowering_counts_for_test,
        };

        let failpoints = [
            AtomicFailpoint::EnvelopeAdmission,
            AtomicFailpoint::SemanticCompilation,
            AtomicFailpoint::MutationPreflight,
            AtomicFailpoint::FinalPreflight,
            AtomicFailpoint::EncodedAdmission,
            AtomicFailpoint::CanonicalOutputAdmission,
            AtomicFailpoint::RevisionAdmission,
            AtomicFailpoint::DurableMetadataAdmission,
        ];
        for (index, failpoint) in failpoints.into_iter().enumerate() {
            let mut engine = transaction_engine();
            engine
                .import_json(
                    r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                    TransactionOrigin::DocumentImport,
                )
                .unwrap();
            select_text(&mut engine, 70_036_200 + index as u64, 0, 3);
            hydrate_import_for_compile_test(&mut engine);
            let before = atomic_audit(&engine);
            let seed_before = engine
                .derived_state
                .as_ref()
                .unwrap()
                .mutation_lookup_seed
                .clone();
            reset_localized_lookup_counts_for_test();
            reset_range_format_lowering_counts_for_test();
            set_atomic_failpoint_for_test(Some(failpoint));

            let error = engine
                .apply_command(
                    70_036_300 + index as u64,
                    TypedCommand::ToggleMark {
                        mark_type: "bold".into(),
                    },
                )
                .unwrap_err();

            set_atomic_failpoint_for_test(None);
            let lookup_counts = take_localized_lookup_counts_for_test();
            let range_counts = take_range_format_lowering_counts_for_test();
            let expected_range_counts = if matches!(
                failpoint,
                AtomicFailpoint::EnvelopeAdmission | AtomicFailpoint::SemanticCompilation
            ) {
                (0, 0, 0, 0)
            } else {
                (0, 0, 1, 0)
            };
            assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{failpoint:?}");
            assert_eq!(
                error.details,
                Some(json!({ "failpoint": failpoint.field_name() })),
                "{failpoint:?}"
            );
            assert_eq!(range_counts, expected_range_counts, "{failpoint:?}");
            assert_eq!(lookup_counts, (0, 0, 0), "{failpoint:?}");
            assert!(Arc::ptr_eq(
                &seed_before,
                &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
            ));
            assert_eq!(atomic_audit(&engine), before, "{failpoint:?}");
        }
    }

    #[test]
    fn prepared_wrap_matches_the_public_planned_transaction_path() {
        let mut prepared = transaction_engine();
        let mut generic = transaction_engine();
        let command = TypedCommand::WrapInList {
            list_type: "bulletList".into(),
            item_type: "listItem".into(),
        };

        let prepared_result = prepared
            .apply_command(70_037, command.clone())
            .unwrap()
            .unwrap();
        let CommandPlan::Transaction(transaction) = generic.plan_command(70_037, command).unwrap()
        else {
            panic!("public wrap planning must produce a transaction")
        };
        let generic_result = generic
            .apply_typed_transaction_with_result(transaction)
            .unwrap();

        assert_eq!(prepared_result, generic_result);
        assert_eq!(prepared.document_json(), generic.document_json());
        assert_eq!(prepared.document_html(), generic.document_html());
        assert_eq!(prepared.resolved_selection(), generic.resolved_selection());
        assert_eq!(prepared.stored_marks(), generic.stored_marks());
        assert_eq!(prepared.can_undo(), generic.can_undo());
        assert_eq!(prepared.can_redo(), generic.can_redo());

        assert_eq!(
            prepared.undo_with_result(70_038).unwrap(),
            generic.undo_with_result(70_038).unwrap()
        );
        assert_eq!(prepared.document_json(), generic.document_json());
        assert_eq!(
            prepared.redo_with_result(70_039).unwrap(),
            generic.redo_with_result(70_039).unwrap()
        );
        assert_eq!(prepared.document_json(), generic.document_json());
    }

    #[test]
    fn derived_state_node_count_refreshes_and_empty_results_use_equivalent_commands() {
        let mut engine = transaction_engine();
        let initial = engine.derived_state.as_ref().unwrap();
        assert_eq!(
            initial.document_node_count,
            crate::editor_state::document_node_count(initial.document.root())
        );

        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"one"}]},{"type":"paragraph","content":[{"type":"text","text":"two"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let refreshed = engine.derived_state.as_ref().unwrap();
        assert_eq!(refreshed.document_revision, engine.revision());
        assert_eq!(
            refreshed.document_node_count,
            crate::editor_state::document_node_count(refreshed.document.root())
        );

        let transaction = TypedTransaction {
            request_id: 991,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: Vec::new(),
            selection_intent: SelectionIntent::Set(SelectionInput::Text {
                anchor: RevisionedPosition {
                    offset: 1,
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::Before,
                },
                head: RevisionedPosition {
                    offset: 1,
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::Before,
                },
            }),
            history_policy: HistoryPolicy::Skip,
        };
        let result = engine
            .apply_typed_transaction_with_result(transaction)
            .unwrap();
        let state = engine.derived_state.as_ref().unwrap();
        let selection = state.legacy_selection();
        assert_eq!(
            result.active_state.commands,
            crate::editor_state::command_applicability(
                &state.document,
                &engine.schema,
                &selection,
                &engine.resource_limits,
            )
        );
    }

    #[test]
    fn utf16_doc_preserves_fresh_client_ids_and_uses_utf16_offsets() {
        let first = utf16_doc();
        let second = utf16_doc();

        assert_eq!(first.offset_kind(), OffsetKind::Utf16);
        assert_eq!(second.offset_kind(), OffsetKind::Utf16);
        assert_ne!(first.client_id(), second.client_id());
    }

    #[test]
    fn validated_import_source_reuses_one_schema_ranked_canonical_result() {
        use crate::yrs_engine::canonical::{
            reset_canonical_artifact_counts_for_test,
            reset_canonical_schema_context_count_for_test, take_canonical_artifact_counts_for_test,
            take_canonical_schema_context_count_for_test,
        };

        let schema = tiptap_schema();
        let limits = ResourceLimits::default();
        let input = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{
                    "type": "text",
                    "text": "ordered",
                    "marks": [{ "type": "bold" }, { "type": "italic" }]
                }]
            }]
        });
        let parsed = from_prosemirror_json(&input, &schema, UnknownTypeMode::Preserve).unwrap();
        let canonical_schema = crate::yrs_engine::canonical::CanonicalSchemaContext::new(&schema);
        let engine = transaction_engine();
        reset_canonical_artifact_counts_for_test();
        reset_canonical_schema_context_count_for_test();
        crate::yrs_engine::observability::reset_full_pass_counts_for_test();

        let input_len = serde_json::to_vec(&input).unwrap().len();
        let validated = ValidatedImportDocument::new(
            parsed,
            &schema,
            &canonical_schema,
            &limits,
            Some(input_len),
        )
        .unwrap();
        let artifact = validated.canonical_artifact.clone();

        assert_eq!(
            validated.canonical_artifact.value(),
            &json!({
                "type": "doc",
                "content": [{
                    "type": "paragraph",
                    "content": [{
                        "type": "text",
                        "text": "ordered",
                        "marks": [{ "type": "bold" }, { "type": "italic" }]
                    }]
                }]
            })
        );
        assert_eq!(
            validated.canonical_artifact.value(),
            &crate::serialize::to_prosemirror_json(&validated.document, &schema)
        );
        let candidate = engine
            .build_candidate_from_document(validated, TransactionOrigin::DocumentImport)
            .unwrap();
        let super::EngineDocumentState::Ready {
            canonical_artifact, ..
        } = candidate.state
        else {
            panic!("validated import candidate must be ready")
        };
        assert!(artifact.ptr_eq(&canonical_artifact));
        assert_eq!(take_canonical_artifact_counts_for_test(), (1, 0));
        assert_eq!(take_canonical_schema_context_count_for_test(), 0);
        let counts = crate::yrs_engine::observability::take_full_pass_counts_for_test();
        assert_eq!(counts.canonical_mark_nodes_visited, 3);
        assert_eq!(counts.canonical_identity_predicate_nodes_visited, 0);
    }

    #[test]
    fn admitted_import_runs_one_validation_certificate_and_render_path() {
        use crate::yrs_engine::observability::{
            reset_full_pass_counts_for_test, take_full_pass_counts_for_test,
        };

        let mut engine = transaction_engine();
        reset_full_pass_counts_for_test();
        crate::render::incremental::reset_cached_render_counts_for_test();

        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"one"}]},{"type":"paragraph","content":[{"type":"text","text":"two"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();

        let passes = take_full_pass_counts_for_test();
        let render = crate::render::incremental::take_cached_render_counts_for_test();
        assert_eq!(passes.import_model_parses, 1);
        assert_eq!(passes.validated_evidence_constructions, 1);
        assert_eq!(passes.validation_certificate_constructions, 1);
        assert_eq!(passes.document_validations, 1);
        assert_eq!(passes.canonical_mark_validation_attempts, 1);
        assert_eq!(passes.canonical_mark_validation_completions, 1);
        assert_eq!(passes.canonical_projections, 1);
        assert_eq!(passes.canonical_serializations, 0);
        assert_eq!(passes.canonical_hashes, 0);
        assert_eq!(
            passes.render_limit_tree_scans, 0,
            "sealed validation evidence should replace the redundant render node/depth scan"
        );
        assert_eq!(
            render.0, 1,
            "the admitted import should build one render cache"
        );

        let artifact = &engine.derived_state.as_ref().unwrap().canonical_artifact;
        let _ = artifact.sha256();
        assert_eq!(take_full_pass_counts_for_test().canonical_hashes, 1);
        let _ = artifact.sha256();
        assert_eq!(take_full_pass_counts_for_test().canonical_hashes, 0);
    }

    #[test]
    fn admitted_import_hydrates_before_seed_consumers_but_not_selection_only_state() {
        let mut typed_input = import_document_with_unavailable_lookup_seed();
        typed_input
            .apply_typed_transaction(insert_transaction(&typed_input, 65_100))
            .unwrap();
        assert!(typed_input
            .derived_state
            .as_ref()
            .unwrap()
            .mutation_lookup_seed
            .is_ready_for_test());

        let mut command = import_document_with_unavailable_lookup_seed();
        command
            .apply_command(65_101, TypedCommand::InsertText { text: "x".into() })
            .unwrap()
            .expect("default-selection command should apply without preparatory selection");
        assert!(command
            .derived_state
            .as_ref()
            .unwrap()
            .mutation_lookup_seed
            .is_ready_for_test());

        let mut selection = import_document_with_unavailable_lookup_seed();
        selection
            .apply_typed_transaction(TypedTransaction {
                request_id: 65_102,
                base_document_revision: selection.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: vec![],
                selection_intent: SelectionIntent::Set(SelectionInput::All),
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap();
        assert!(selection
            .derived_state
            .as_ref()
            .unwrap()
            .mutation_lookup_seed
            .is_unavailable_for_test());

        let mut rich_local_api = import_document_with_unavailable_lookup_seed();
        rich_local_api
            .apply_typed_transaction_with_result(TypedTransaction {
                request_id: 65_103,
                base_document_revision: rich_local_api.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: vec![],
                selection_intent: SelectionIntent::Preserve,
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap();
        assert!(rich_local_api
            .derived_state
            .as_ref()
            .unwrap()
            .mutation_lookup_seed
            .is_unavailable_for_test());

        let mut history = import_document_with_unavailable_lookup_seed();
        assert!(history.undo(65_104).unwrap().is_none());
        assert!(history
            .derived_state
            .as_ref()
            .unwrap()
            .mutation_lookup_seed
            .is_unavailable_for_test());
        history
            .apply_command(65_105, TypedCommand::InsertText { text: "x".into() })
            .unwrap()
            .unwrap();
        force_lookup_seed_unavailable(&mut history);
        let unavailable_before_undo =
            Arc::clone(&history.derived_state.as_ref().unwrap().mutation_lookup_seed);
        assert!(history.undo(65_106).unwrap().is_some());
        assert!(!Arc::ptr_eq(
            &unavailable_before_undo,
            &history.derived_state.as_ref().unwrap().mutation_lookup_seed
        ));
        let unavailable_before_redo =
            Arc::clone(&history.derived_state.as_ref().unwrap().mutation_lookup_seed);
        assert!(history.redo(65_107).unwrap().is_some());
        assert!(!Arc::ptr_eq(
            &unavailable_before_redo,
            &history.derived_state.as_ref().unwrap().mutation_lookup_seed
        ));
    }

    #[test]
    fn deferred_insert_shape_and_output_bound_eligibility_is_exact() {
        let exact = deferred_insert_fixture(DeferredInsertCase::StrictInteriorEqualMarks);
        assert_eq!(
            exact.execution_admission_kind(),
            ExecutionAdmissionKind::Deferred
        );

        for case in [
            DeferredInsertCase::Empty,
            DeferredInsertCase::LeafBoundary,
            DeferredInsertCase::MarkMismatch,
            DeferredInsertCase::StructuralGrowth,
            DeferredInsertCase::UnavailableUpperBound,
            DeferredInsertCase::OverflowingUpperBound,
            DeferredInsertCase::OneOverOutputLimit,
        ] {
            assert_eq!(
                deferred_insert_fixture(case).execution_admission_kind(),
                ExecutionAdmissionKind::Eager,
                "{case:?}",
            );
        }
    }

    #[test]
    fn eager_semantic_errors_precede_staged_hydration_failure() {
        use crate::yrs_engine::mutation::{
            set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
        };

        for case in eager_pre_admission_error_cases() {
            let mut engine = case.engine;
            let before = atomic_audit(&engine);
            set_lookup_seed_hydration_failpoint_for_test(Some(
                LookupSeedHydrationFailpoint::InitialReservation,
            ));
            let error = engine
                .apply_command(case.request_id, case.command)
                .unwrap_err();
            set_lookup_seed_hydration_failpoint_for_test(None);
            assert_eq!(error, case.expected_error, "{}", case.name);
            assert_eq!(atomic_audit(&engine), before, "{}", case.name);
        }
    }

    #[test]
    fn first_imported_deferred_insert_uses_two_serializations_two_hashes_once() {
        use crate::yrs_engine::mutation::{
            reset_localized_lookup_counts_for_test, take_localized_lookup_counts_for_test,
        };
        use crate::yrs_engine::observability::{
            reset_full_pass_counts_for_test, reset_prepared_admission_counts_for_test,
            take_full_pass_counts_for_test, take_prepared_admission_counts_for_test,
        };

        let mut engine = import_document_with_unavailable_lookup_seed();
        select_text(&mut engine, 65_199, 2, 2);
        assert!(engine
            .derived_state
            .as_ref()
            .unwrap()
            .mutation_lookup_seed
            .is_unavailable_for_test());
        reset_full_pass_counts_for_test();
        reset_prepared_admission_counts_for_test();
        reset_localized_lookup_counts_for_test();

        engine
            .apply_command(65_200, TypedCommand::InsertText { text: "x".into() })
            .unwrap()
            .expect("strict-interior imported insert should apply");

        let passes = take_full_pass_counts_for_test();
        assert_eq!(passes.planner_simulations, 1);
        assert_eq!(passes.document_validations, 1);
        assert_eq!(passes.canonical_serializations, 2);
        assert_eq!(passes.canonical_hashes, 2);
        assert_eq!(take_localized_lookup_counts_for_test(), (0, 1, 1));
        let admission = take_prepared_admission_counts_for_test();
        assert_eq!(admission.staged_seed_preparations, 1);
        assert_eq!(admission.installed_base_seed_publications, 0);
    }

    #[test]
    fn public_insert_uses_eager_admission_after_admissible_resource_limit_change() {
        use crate::yrs_engine::observability::{
            reset_full_pass_counts_for_test, reset_prepared_admission_counts_for_test,
            take_full_pass_counts_for_test, take_prepared_admission_counts_for_test,
        };

        let mut engine = import_document_with_unavailable_lookup_seed();
        select_text(&mut engine, 65_201, 2, 2);
        engine.resource_limits.max_input_bytes -= 1;
        let changed_limits = engine.resource_limits.clone();
        let mut preconfigured = transaction_engine();
        preconfigured.resource_limits = changed_limits;
        preconfigured
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        select_text(&mut preconfigured, 65_201, 2, 2);
        let command = TypedCommand::InsertText { text: "x".into() };
        let preparation = std::cell::RefCell::new(None);
        assert!(matches!(
            engine
                .plan_command_internal(65_202, command.clone(), Some(&preparation))
                .unwrap(),
            CommandPlan::Transaction(_)
        ));
        assert!(matches!(
            preparation.into_inner().unwrap().execution_admission,
            crate::yrs_engine::prepared_admission::ExecutionSemanticAdmission::Eager(_)
        ));
        let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
        reset_full_pass_counts_for_test();
        reset_prepared_admission_counts_for_test();

        let result = engine.apply_command(65_202, command).unwrap().unwrap();
        let passes = take_full_pass_counts_for_test();
        let counts = take_prepared_admission_counts_for_test();
        let preconfigured_result = preconfigured
            .apply_command(65_202, TypedCommand::InsertText { text: "x".into() })
            .unwrap()
            .unwrap();

        assert!(result.changed);
        assert_eq!(passes.planner_simulations, 1);
        assert_eq!(passes.document_validations, 4);
        assert_eq!(result, preconfigured_result);
        assert_eq!(engine.document_json(), preconfigured.document_json());
        assert_eq!(engine.document_html(), preconfigured.document_html());
        assert_eq!(
            engine.resolved_selection(),
            preconfigured.resolved_selection()
        );
        assert!(!Arc::ptr_eq(
            &installed,
            &engine.derived_state.as_ref().unwrap().mutation_lookup_seed,
        ));
        assert!(engine
            .derived_state
            .as_ref()
            .unwrap()
            .mutation_lookup_seed
            .is_ready_for_test());
        assert_eq!(counts.staged_seed_preparations, 1);
        assert_eq!(counts.installed_base_seed_publications, 0);
    }

    #[test]
    fn private_prepared_command_orchestrator_finalizes_deferred_admission_once() {
        use crate::yrs_engine::mutation::{
            reset_localized_lookup_counts_for_test, take_localized_lookup_counts_for_test,
        };
        use crate::yrs_engine::observability::{
            reset_full_pass_counts_for_test, reset_prepared_admission_counts_for_test,
            take_full_pass_counts_for_test, take_prepared_admission_counts_for_test,
        };
        use crate::yrs_engine::TransactionCommit;

        let mut engine = import_document_with_unavailable_lookup_seed();
        let mut public = import_document_with_unavailable_lookup_seed();
        select_text(&mut engine, 65_260, 2, 2);
        select_text(&mut public, 65_260, 2, 2);
        let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
        let preparation = std::cell::RefCell::new(None);
        reset_full_pass_counts_for_test();
        reset_prepared_admission_counts_for_test();
        reset_localized_lookup_counts_for_test();

        let CommandPlan::Transaction(transaction) = engine
            .plan_command_internal(
                65_261,
                TypedCommand::InsertText { text: "x".into() },
                Some(&preparation),
            )
            .unwrap()
        else {
            panic!("strict-interior imported insert must produce a transaction")
        };
        let proof = preparation
            .into_inner()
            .expect("strict-interior imported insert must retain its exact proof");
        assert!(matches!(
            &proof.execution_admission,
            crate::yrs_engine::prepared_admission::ExecutionSemanticAdmission::Deferred(_)
        ));
        let (commit, result) = engine
            .apply_prepared_command_transaction(
                transaction,
                proof,
                true,
                &mut OutboundUpdateSink::detached(),
            )
            .unwrap();
        let result = result.expect("changed command must return a result");
        let authority_counts = take_compiled_commit_authority_counts_for_test();
        let passes = take_full_pass_counts_for_test();
        let admission = take_prepared_admission_counts_for_test();
        assert_eq!(passes.planner_simulations, 1);
        assert_eq!(passes.document_validations, 1);
        assert_eq!(passes.canonical_serializations, 2);
        assert_eq!(passes.canonical_hashes, 2);
        assert_eq!(take_localized_lookup_counts_for_test(), (0, 1, 1));
        assert_eq!(admission.staged_seed_preparations, 1);
        assert_eq!(admission.staged_identity_materializations, 1);
        assert_eq!(admission.installed_base_seed_publications, 0);
        assert_eq!(admission.deferred_capsules_created, 1);
        assert_eq!(admission.deferred_capsules_finalized, 1);
        assert_eq!(authority_counts, (1, 1));
        assert!(!Arc::ptr_eq(
            &installed,
            &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
        ));
        assert!(engine
            .derived_state
            .as_ref()
            .unwrap()
            .mutation_lookup_seed
            .is_ready_for_test());
        assert_eq!(
            commit,
            TransactionCommit {
                request_id: result.request_id,
                changed: result.changed,
                document_revision: result.document_revision,
                state_revision: result.state_revision,
                origin: result.origin,
            }
        );

        let public_result = public
            .apply_command(65_261, TypedCommand::InsertText { text: "x".into() })
            .unwrap()
            .unwrap();
        assert_eq!(result, public_result);
        assert_eq!(engine.document_json(), public.document_json());
        assert_eq!(engine.resolved_selection(), public.resolved_selection());
        assert_eq!(engine.stored_marks(), public.stored_marks());
        assert_eq!(engine.can_undo(), public.can_undo());
        assert_eq!(engine.can_redo(), public.can_redo());
        let private_undo = engine.undo(65_262).unwrap().unwrap();
        let public_undo = public.undo(65_262).unwrap().unwrap();
        assert_eq!(private_undo, public_undo);
        assert_eq!(engine.document_json(), public.document_json());
        assert_eq!(engine.resolved_selection(), public.resolved_selection());
        assert_eq!(engine.stored_marks(), public.stored_marks());
        assert_eq!(engine.can_undo(), public.can_undo());
        assert_eq!(engine.can_redo(), public.can_redo());
    }

    #[test]
    fn first_imported_prepared_insert_traverses_each_history_document_once() {
        use crate::model::{
            reset_history_snapshot_retained_bytes_traversals_for_test,
            take_history_snapshot_retained_bytes_traversals_for_test,
        };

        let mut engine = import_document_with_unavailable_lookup_seed();
        select_text(&mut engine, 65_263, 2, 2);
        reset_history_snapshot_retained_bytes_traversals_for_test();

        engine
            .apply_command(65_264, TypedCommand::InsertText { text: "x".into() })
            .unwrap()
            .expect("strict-interior imported insert must apply");

        assert_eq!(
            take_history_snapshot_retained_bytes_traversals_for_test(),
            2,
            "history admission must traverse the before and after source documents once each"
        );
    }

    #[test]
    fn first_imported_prepared_insert_uses_localized_history_render_evidence() {
        use crate::render::incremental::{
            reset_cached_render_counts_for_test, reset_localized_render_transition_counts_for_test,
            take_cached_render_counts_for_test, take_localized_render_transition_counts_for_test,
        };
        use crate::yrs_engine::observability::{
            reset_full_pass_counts_for_test, take_full_pass_counts_for_test,
        };

        let mut engine = import_document_with_unavailable_lookup_seed();
        select_text(&mut engine, 65_265, 2, 2);
        reset_full_pass_counts_for_test();
        reset_cached_render_counts_for_test();
        reset_localized_render_transition_counts_for_test();

        engine
            .apply_command(65_266, TypedCommand::InsertText { text: "x".into() })
            .unwrap()
            .expect("strict-interior imported insert must apply");

        let passes = take_full_pass_counts_for_test();
        let localized = take_localized_render_transition_counts_for_test();
        assert_eq!((passes.render_limit_tree_scans, localized), (0, (1, 1, 0)));
        assert_eq!(
            (
                passes.position_map_clones,
                passes.position_map_compactions,
                passes.rendered_text_derivations,
            ),
            (1, 1, 0),
            "sealed strict-interior evidence must incrementally derive the candidate map and text",
        );
        assert_eq!(take_cached_render_counts_for_test(), (0, 1, 1, 0, 0));
    }

    #[test]
    fn tampered_localized_history_render_evidence_falls_back_with_exact_results() {
        use crate::render::incremental::{
            reset_cached_render_counts_for_test, reset_localized_render_transition_counts_for_test,
            take_cached_render_counts_for_test, take_localized_render_transition_counts_for_test,
        };
        use crate::yrs_engine::observability::{
            reset_full_pass_counts_for_test, take_full_pass_counts_for_test,
        };
        use crate::yrs_engine::prepared_admission::{
            DeferredCommandAdmission, ExecutionSemanticAdmission,
        };

        for case in DeferredCommandAdmission::history_render_tamper_cases_for_test() {
            let mut actual = import_document_with_unavailable_lookup_seed();
            let mut expected = import_document_with_unavailable_lookup_seed();
            select_text(&mut actual, 65_267, 2, 2);
            select_text(&mut expected, 65_267, 2, 2);
            let command = TypedCommand::InsertText { text: "x".into() };
            let preparation = std::cell::RefCell::new(None);
            let CommandPlan::Transaction(transaction) = actual
                .plan_command_internal(65_268, command.clone(), Some(&preparation))
                .unwrap()
            else {
                panic!("strict-interior imported insert must produce a transaction")
            };
            let mut proof = preparation.into_inner().unwrap();
            let ExecutionSemanticAdmission::Deferred(deferred) = &mut proof.execution_admission
            else {
                panic!("strict-interior imported insert must retain deferred evidence")
            };
            deferred.tamper_history_render_for_test(case);
            reset_full_pass_counts_for_test();
            reset_cached_render_counts_for_test();
            reset_localized_render_transition_counts_for_test();

            let actual_result = actual
                .apply_prepared_command_transaction(
                    transaction,
                    proof,
                    true,
                    &mut OutboundUpdateSink::detached(),
                )
                .unwrap()
                .1
                .unwrap();
            let passes = take_full_pass_counts_for_test();
            let cached = take_cached_render_counts_for_test();
            let localized = take_localized_render_transition_counts_for_test();
            let expected_result = expected.apply_command(65_268, command).unwrap().unwrap();

            assert_eq!(actual_result, expected_result, "{case}");
            assert_eq!(actual.document_json(), expected.document_json(), "{case}");
            assert_eq!(
                actual.resolved_selection(),
                expected.resolved_selection(),
                "{case}"
            );
            assert_eq!(actual.can_undo(), expected.can_undo(), "{case}");
            assert_eq!(passes.render_limit_tree_scans, 1, "{case}");
            assert_eq!(cached, (0, 1, 1, 0, 0), "{case}");
            assert_eq!(localized, (1, 0, 1), "{case}");
        }
    }

    #[test]
    fn localized_history_render_errors_fall_back_with_exact_results() {
        use crate::render::incremental::{
            reset_cached_render_counts_for_test, reset_localized_render_transition_counts_for_test,
            set_localized_render_failure_stage_for_test, take_cached_render_counts_for_test,
            take_localized_render_transition_counts_for_test, LocalizedRenderFailureStage,
        };
        use crate::yrs_engine::observability::{
            reset_full_pass_counts_for_test, take_full_pass_counts_for_test,
        };

        for stage in [
            LocalizedRenderFailureStage::Allocation,
            LocalizedRenderFailureStage::Resource,
            LocalizedRenderFailureStage::Position,
            LocalizedRenderFailureStage::Invariant,
        ] {
            let mut actual = import_document_with_unavailable_lookup_seed();
            let mut expected = import_document_with_unavailable_lookup_seed();
            let two_blocks = r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]},{"type":"paragraph","content":[{"type":"text","text":"suffix"}]}]}"#;
            actual
                .import_json(two_blocks, TransactionOrigin::DocumentImport)
                .unwrap();
            expected
                .import_json(two_blocks, TransactionOrigin::DocumentImport)
                .unwrap();
            select_text(&mut actual, 65_269, 2, 2);
            select_text(&mut expected, 65_269, 2, 2);
            reset_full_pass_counts_for_test();
            reset_cached_render_counts_for_test();
            reset_localized_render_transition_counts_for_test();
            set_localized_render_failure_stage_for_test(Some(stage));

            let actual_result = actual
                .apply_command(65_270, TypedCommand::InsertText { text: "x".into() })
                .unwrap()
                .unwrap();
            set_localized_render_failure_stage_for_test(None);
            let passes = take_full_pass_counts_for_test();
            let cached = take_cached_render_counts_for_test();
            let localized = take_localized_render_transition_counts_for_test();
            let expected_result = expected
                .apply_command(65_270, TypedCommand::InsertText { text: "x".into() })
                .unwrap()
                .unwrap();

            assert_eq!(actual_result, expected_result, "{stage:?}");
            assert_eq!(
                actual.document_json(),
                expected.document_json(),
                "{stage:?}"
            );
            assert_eq!(
                actual.resolved_selection(),
                expected.resolved_selection(),
                "{stage:?}"
            );
            assert_eq!(actual.can_undo(), expected.can_undo(), "{stage:?}");
            assert_eq!(passes.render_limit_tree_scans, 1, "{stage:?}");
            assert_eq!(cached, (0, 1, 1, 0, 0), "{stage:?}");
            assert_eq!(localized, (1, 0, 1), "{stage:?}");
        }
    }

    #[test]
    fn private_prepared_eager_noninsert_uses_staged_context_without_identity() {
        use crate::yrs_engine::observability::{
            reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
        };
        use crate::yrs_engine::TransactionCommit;

        let mut engine = import_document_with_unavailable_lookup_seed();
        let mut public = import_document_with_unavailable_lookup_seed();
        select_text(&mut engine, 65_263, 0, 2);
        select_text(&mut public, 65_263, 0, 2);
        let preparation = std::cell::RefCell::new(None);
        reset_prepared_admission_counts_for_test();
        let command = TypedCommand::ToggleMark {
            mark_type: "bold".into(),
        };
        let CommandPlan::Transaction(transaction) = engine
            .plan_command_internal(65_264, command.clone(), Some(&preparation))
            .unwrap()
        else {
            panic!("range mark command must produce a transaction")
        };
        let proof = preparation.into_inner().unwrap();
        assert!(matches!(
            &proof.execution_admission,
            crate::yrs_engine::prepared_admission::ExecutionSemanticAdmission::Eager(_)
        ));

        let (commit, result) = engine
            .apply_prepared_command_transaction(
                transaction,
                proof,
                true,
                &mut OutboundUpdateSink::detached(),
            )
            .unwrap();
        let result = result.unwrap();
        let admission = take_prepared_admission_counts_for_test();
        assert_eq!(admission.staged_seed_preparations, 1);
        assert_eq!(admission.staged_identity_materializations, 0);
        assert_eq!(admission.installed_base_seed_publications, 0);
        assert_eq!(
            commit,
            TransactionCommit {
                request_id: result.request_id,
                changed: result.changed,
                document_revision: result.document_revision,
                state_revision: result.state_revision,
                origin: result.origin,
            }
        );

        let public_result = public.apply_command(65_264, command).unwrap().unwrap();
        assert_eq!(result, public_result);
        assert_eq!(engine.document_json(), public.document_json());
        assert_eq!(engine.resolved_selection(), public.resolved_selection());
        assert_eq!(engine.stored_marks(), public.stored_marks());
        assert_eq!(engine.can_undo(), public.can_undo());
        assert_eq!(engine.can_redo(), public.can_redo());
    }

    #[test]
    fn private_prepared_history_error_precedes_staged_hydration_failure() {
        use crate::yrs_engine::mutation::{
            set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
        };
        use crate::yrs_engine::observability::{
            reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
        };

        let limits = crate::yrs_engine::EditingLimits {
            max_derived_output_bytes: 100,
            ..crate::yrs_engine::EditingLimits::default()
        };
        let mut engine = transaction_engine_with_editing_limits(limits);
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        select_text(&mut engine, 65_265, 2, 2);
        engine.derived_state.as_mut().unwrap().canonical_artifact = engine
            .derived_state
            .as_ref()
            .unwrap()
            .canonical_artifact
            .with_admission_upper_bound_for_test(usize::MAX);
        let expected_actual =
            super::history_metadata_bytes(engine.stored_marks(), "prosemirror") * 2;
        let preparation = std::cell::RefCell::new(None);
        let CommandPlan::Transaction(transaction) = engine
            .plan_command_internal(
                65_266,
                TypedCommand::InsertText { text: "x".into() },
                Some(&preparation),
            )
            .unwrap()
        else {
            panic!("insert command must produce a transaction")
        };
        let proof = preparation.into_inner().unwrap();
        let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
        let before = atomic_audit(&engine);
        reset_prepared_admission_counts_for_test();
        set_lookup_seed_hydration_failpoint_for_test(Some(
            LookupSeedHydrationFailpoint::InitialReservation,
        ));
        let error = engine
            .apply_prepared_command_transaction(
                transaction,
                proof,
                true,
                &mut OutboundUpdateSink::detached(),
            )
            .unwrap_err();
        set_lookup_seed_hydration_failpoint_for_test(None);

        assert_eq!(
            error,
            crate::yrs_engine::OperationError::document_limit_exceeded(
                65_266,
                None,
                "maxDerivedOutputBytes",
                100,
                expected_actual as u64,
            )
        );
        assert_eq!(atomic_audit(&engine), before);
        assert!(Arc::ptr_eq(
            &installed,
            &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
        ));
        let admission = take_prepared_admission_counts_for_test();
        assert_eq!(admission.staged_seed_preparations, 0);
        assert_eq!(admission.installed_base_seed_publications, 0);
    }

    #[test]
    fn private_prepared_deferred_compiler_failure_is_prewrite_and_atomic() {
        use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};
        use crate::yrs_engine::observability::{
            reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
        };

        let mut engine = import_document_with_unavailable_lookup_seed();
        select_text(&mut engine, 65_267, 2, 2);
        let preparation = std::cell::RefCell::new(None);
        let CommandPlan::Transaction(transaction) = engine
            .plan_command_internal(
                65_268,
                TypedCommand::InsertText { text: "x".into() },
                Some(&preparation),
            )
            .unwrap()
        else {
            panic!("strict-interior imported insert must produce a transaction")
        };
        let proof = preparation.into_inner().unwrap();
        assert!(matches!(
            &proof.execution_admission,
            crate::yrs_engine::prepared_admission::ExecutionSemanticAdmission::Deferred(_)
        ));
        let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
        let before = atomic_audit(&engine);
        reset_prepared_admission_counts_for_test();
        set_atomic_failpoint_for_test(Some(AtomicFailpoint::SemanticCompilation));
        let error = engine
            .apply_prepared_command_transaction(
                transaction,
                proof,
                true,
                &mut OutboundUpdateSink::detached(),
            )
            .unwrap_err();
        set_atomic_failpoint_for_test(None);

        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
        assert_eq!(atomic_audit(&engine), before);
        assert!(Arc::ptr_eq(
            &installed,
            &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
        ));
        let admission = take_prepared_admission_counts_for_test();
        assert_eq!(admission.staged_seed_preparations, 1);
        assert_eq!(admission.staged_identity_materializations, 1);
        assert_eq!(admission.installed_base_seed_publications, 0);
        assert_eq!(admission.deferred_capsules_finalized, 1);
    }

    #[test]
    fn eager_non_insert_first_mutations_do_not_materialize_base_identity() {
        use crate::yrs_engine::observability::{
            reset_full_pass_counts_for_test, reset_prepared_admission_counts_for_test,
            take_full_pass_counts_for_test, take_prepared_admission_counts_for_test,
        };

        let mut toggle = import_document_with_unavailable_lookup_seed();
        select_text(&mut toggle, 65_201, 0, 2);
        reset_full_pass_counts_for_test();
        reset_prepared_admission_counts_for_test();
        toggle
            .apply_command(
                65_202,
                TypedCommand::ToggleMark {
                    mark_type: "bold".into(),
                },
            )
            .unwrap()
            .unwrap();
        let toggle_passes = take_full_pass_counts_for_test();
        let toggle_admission = take_prepared_admission_counts_for_test();
        assert_eq!(toggle_passes.canonical_serializations, 3);
        assert_eq!(toggle_passes.canonical_hashes, 2);
        assert_eq!(toggle_admission.staged_identity_materializations, 0);

        let mut wrap = import_document_with_unavailable_lookup_seed();
        reset_full_pass_counts_for_test();
        reset_prepared_admission_counts_for_test();
        wrap.apply_command(
            65_203,
            TypedCommand::WrapInList {
                list_type: "bulletList".into(),
                item_type: "listItem".into(),
            },
        )
        .unwrap()
        .unwrap();
        let wrap_passes = take_full_pass_counts_for_test();
        let wrap_admission = take_prepared_admission_counts_for_test();
        assert_eq!(wrap_passes.canonical_serializations, 3);
        assert_eq!(wrap_passes.canonical_hashes, 2);
        assert_eq!(wrap_admission.staged_identity_materializations, 0);

        let mut undo = import_document_with_unavailable_lookup_seed();
        undo.apply_command(65_204, TypedCommand::InsertText { text: "x".into() })
            .unwrap()
            .unwrap();
        force_lookup_seed_unavailable(&mut undo);
        reset_full_pass_counts_for_test();
        reset_prepared_admission_counts_for_test();
        undo.undo(65_205).unwrap().unwrap();
        let undo_passes = take_full_pass_counts_for_test();
        let undo_admission = take_prepared_admission_counts_for_test();
        assert_eq!(undo_passes.canonical_serializations, 0);
        assert_eq!(undo_passes.canonical_hashes, 0);
        assert_eq!(undo_admission.staged_identity_materializations, 0);

        let mut redo = import_document_with_unavailable_lookup_seed();
        redo.apply_command(65_206, TypedCommand::InsertText { text: "x".into() })
            .unwrap()
            .unwrap();
        redo.undo(65_207).unwrap().unwrap();
        force_lookup_seed_unavailable(&mut redo);
        reset_full_pass_counts_for_test();
        reset_prepared_admission_counts_for_test();
        redo.redo(65_208).unwrap().unwrap();
        let redo_passes = take_full_pass_counts_for_test();
        let redo_admission = take_prepared_admission_counts_for_test();
        assert_eq!(redo_passes.canonical_serializations, 0);
        assert_eq!(redo_passes.canonical_hashes, 0);
        assert_eq!(redo_admission.staged_identity_materializations, 0);
    }

    #[test]
    fn staged_authority_supplies_every_unavailable_seed_consumer_without_installed_reads() {
        use crate::yrs_engine::observability::{
            reset_full_pass_counts_for_test, reset_prepared_admission_counts_for_test,
            take_full_pass_counts_for_test, take_prepared_admission_counts_for_test,
        };

        reset_prepared_admission_counts_for_test();
        let (engine, deferred, mut context, transaction, expected_document) =
            deferred_finalization_fixture();
        let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
        engine.prepare_mutation_identity(&mut context).unwrap();
        reset_full_pass_counts_for_test();

        let prepared = engine
            .finalize_deferred_for_test(deferred, &context, &transaction, &expected_document)
            .unwrap();
        let state = engine.derived_state.as_ref().unwrap();
        let cached = state.compilation_view();
        let txn = engine.doc.transact();
        let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
        let authority = context
            .authority(
                crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                    request_id: transaction.request_id,
                    installed: state,
                    txn: &txn,
                    fragment: &fragment,
                    fragment_name: &engine.fragment_name,
                    schema_fingerprint: &engine.schema_fingerprint,
                    resource_limits: &engine.resource_limits,
                    editing_limits: &engine.editing_limits,
                    max_length: engine.max_length,
                    document_revision: engine.revision,
                    state_revision: engine.state_revision,
                    yrs_state_epoch: engine.yrs_state_epoch,
                },
            )
            .unwrap();
        assert!(installed.is_unavailable_for_test());
        assert!(authority.lookup_seed().is_ready_for_test());
        assert!(!Arc::ptr_eq(&installed, authority.lookup_seed()));

        let format_from = crate::yrs_engine::position::editor_offset_to_doc_pos(
            0,
            EditorOffsetKind::Scalar,
            &state.rendered_text,
            &state.position_map,
            &state.document,
        )
        .unwrap();
        let format_to = crate::yrs_engine::position::editor_offset_to_doc_pos(
            2,
            EditorOffsetKind::Scalar,
            &state.rendered_text,
            &state.position_map,
            &state.document,
        )
        .unwrap();
        let format_block = state
            .position_map
            .find_block_for_doc_pos(format_from)
            .and_then(|index| state.position_map.block(index))
            .unwrap();
        let format_locator = crate::yrs_engine::mutation::LocalizedFormatLocator::mint(
            &state.document,
            &format_block.node_path,
            format_from,
            format_to,
            authority.lookup_seed().as_ref(),
            &txn,
            &fragment,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            &engine.schema_fingerprint,
            engine.yrs_state_epoch,
            engine.revision,
        )
        .expect("staged authority mints a current localized format locator");
        assert!(
            crate::yrs_engine::mutation::LocalizedFormatCompiler::try_new(
                transaction.request_id,
                &txn,
                &fragment,
                &engine.schema,
                usize::MAX,
                engine.resource_limits.max_input_bytes,
                0,
                format_locator,
                &engine.schema_fingerprint,
                engine.yrs_state_epoch,
                engine.revision,
            )
            .unwrap()
            .is_some()
        );

        let first_child = state
            .document
            .root()
            .content()
            .and_then(|content| content.child(0))
            .unwrap()
            .clone();
        let root_replacement = crate::yrs_engine::StructuralReplacement::new(
            Vec::new(),
            0,
            1,
            crate::model::Fragment::from(vec![first_child]),
            Selection::cursor(0),
        );
        let root_locator = crate::yrs_engine::mutation::LocalizedRootWindowLocator::mint(
            transaction.request_id,
            &state.document,
            &state.document,
            &root_replacement,
            authority.lookup_seed().as_ref(),
            &txn,
            &fragment,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            &engine.schema_fingerprint,
            engine.yrs_state_epoch,
            engine.revision,
        )
        .unwrap()
        .expect("staged authority mints a current localized root-window locator");
        assert!(
            crate::yrs_engine::mutation::LocalizedRootWindowCompiler::try_new(
                transaction.request_id,
                &txn,
                &fragment,
                &engine.schema,
                usize::MAX,
                engine.resource_limits.max_input_bytes,
                0,
                root_locator,
            )
            .unwrap()
            .is_some()
        );

        let mut compiled =
            crate::yrs_engine::compiler::compile_prepared_transaction_with_yrs_and_stored_marks(
                crate::yrs_engine::compiler::CompilationContext {
                    document: cached.document,
                    selection: Some(cached.selection),
                    schema: &engine.schema,
                    resource_limits: &engine.resource_limits,
                    editing_limits: &engine.editing_limits,
                    document_revision: engine.revision,
                    max_length: engine.max_length,
                },
                transaction.clone(),
                &txn,
                &fragment,
                crate::yrs_engine::compiler::StoredMarksCompilationContext {
                    stored_marks: state.stored_marks.as_deref(),
                    resolved_selection: &state.resolved_selection,
                    relative_selection: &state.relative_selection,
                },
                crate::yrs_engine::compiler::PreparedSemanticContext {
                    admission: &prepared,
                    expected_preview: &expected_document,
                    yrs_state_epoch: engine.yrs_state_epoch,
                    state_revision: engine.state_revision,
                    schema_fingerprint: &engine.schema_fingerprint,
                },
                crate::yrs_engine::compiler::EngineCompilationView {
                    cached,
                    authority: &authority,
                    state_revision: engine.state_revision,
                    schema_fingerprint: &engine.schema_fingerprint,
                    yrs_state_epoch: engine.yrs_state_epoch,
                },
            )
            .unwrap();
        assert!(compiled.localized_semantic_used);
        assert!(compiled.localized_insert_admission.is_some());
        assert!(compiled.prepared_derived_evidence.is_some());
        assert!(compiled.mutation_lookup_transition.is_some());

        let admission = compiled.localized_insert_admission.as_ref().unwrap();
        let crate::yrs_engine::compiler::StoredMarksPlan::Set(stored_marks) =
            &compiled.stored_marks_plan
        else {
            panic!("localized compiler seals stored marks")
        };
        let active_transition = state
            .prepare_active_state_transition(
                transaction.request_id,
                &authority,
                admission,
                &compiled.preview,
                admission.operation_result_selection(),
                stored_marks.as_deref(),
                &engine.resource_limits,
                &engine.editing_limits,
                engine.max_length,
                engine.yrs_state_epoch,
            )
            .unwrap();
        let structural = admission.active_state_structural_seal();
        assert!(state
            .validate_active_state_transition(
                &authority,
                &active_transition,
                &structural,
                &compiled.preview,
                admission.operation_result_selection(),
                stored_marks.as_deref(),
                &engine.resource_limits,
                &engine.editing_limits,
                engine.max_length,
                engine.yrs_state_epoch,
            )
            .is_some());

        let selection_seal =
            crate::yrs_engine::compiler::PreparedSelectionMutationSeal::capture(&compiled)
                .expect("localized insert captures its prepared selection seal");
        assert!(selection_seal.matches(&compiled, &authority));

        let evidence = compiled.prepared_derived_evidence.take().unwrap();
        let derivations = compiled.preview_derivations.as_ref().unwrap();
        let render_transition = evidence
            .prepare_localized_render_transition(
                state,
                &compiled.preview,
                derivations,
                &compiled.affected_top_level_blocks,
                &engine.schema,
                &engine.schema_fingerprint,
                &engine.resource_limits,
                &engine.editing_limits,
                engine.max_length,
            )
            .expect("localized render proof remains current")
            .unwrap();
        let next_document_revision = engine.revision.checked_add(1).unwrap();
        let next_state_revision = engine.state_revision.checked_add(1).unwrap();
        let next_yrs_state_epoch = engine.yrs_state_epoch.checked_add(1).unwrap();
        assert!(evidence
            .finalize(
                &authority,
                &compiled.preview,
                compiled.canonical_artifact.as_ref().unwrap(),
                derivations,
                &render_transition.cache,
                &engine.resource_limits,
                &engine.editing_limits,
                engine.max_length,
                &engine.schema_fingerprint,
                next_document_revision,
                next_state_revision,
                next_yrs_state_epoch,
            )
            .is_some());

        let next_seed = engine
            .prepare_mutation_lookup_transition_with_authority(
                transaction.request_id,
                &authority,
                compiled.mutation_lookup_transition.as_ref().unwrap(),
                &txn,
                &fragment,
                &compiled.preview,
                compiled.canonical_artifact.as_ref().unwrap(),
                next_yrs_state_epoch,
                next_document_revision,
            )
            .unwrap();
        assert!(next_seed.is_ready_for_test());
        assert!(!Arc::ptr_eq(&installed, &next_seed));
        let installed_adapter =
            crate::yrs_engine::prepared_admission::InstalledDerivedStateAuthority::new(state);
        assert!(
            crate::yrs_engine::prepared_admission::DerivedStateAuthority::lookup_seed(
                &installed_adapter,
                transaction.request_id,
            )
            .is_err()
        );
        assert!(Arc::ptr_eq(
            &installed,
            &engine.derived_state.as_ref().unwrap().mutation_lookup_seed,
        ));
        let passes = take_full_pass_counts_for_test();
        assert_eq!(passes.document_validations, 0);
        let counts = take_prepared_admission_counts_for_test();
        assert_eq!(counts.staged_seed_preparations, 1);
        assert_eq!(counts.installed_base_seed_publications, 0);
    }

    #[test]
    fn staged_authority_rejects_installed_substitution_and_live_seal_drift_before_transition() {
        use crate::yrs_engine::observability::{
            reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
        };

        for case in [
            "request",
            "store",
            "fragment",
            "schema",
            "resource_limits",
            "editing_limits",
            "max_length",
            "document_revision",
            "state_revision",
            "epoch",
            "identity",
        ] {
            let mut engine = import_document_with_unavailable_lookup_seed();
            let installed =
                Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
            reset_prepared_admission_counts_for_test();
            let mut context = engine.prepare_mutation_lookup_seed(65_250).unwrap();
            engine.prepare_mutation_identity(&mut context).unwrap();

            if case == "identity" {
                let state = engine.derived_state.as_mut().unwrap();
                state.canonical_artifact = state
                    .canonical_artifact
                    .schema_context()
                    .derive(&state.document)
                    .unwrap();
            }
            let before = atomic_audit(&engine);
            let state = engine.derived_state.as_ref().unwrap();
            let txn = engine.doc.transact();
            let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
            let foreign = transaction_engine();
            let foreign_txn = foreign.doc.transact();
            let foreign_fragment = foreign_txn
                .get_xml_fragment(foreign.fragment_name.as_str())
                .unwrap();
            let mut drifted_resources = engine.resource_limits.clone();
            drifted_resources.max_input_bytes = drifted_resources
                .max_input_bytes
                .checked_sub(1)
                .expect("fixture resource limit is positive");
            let mut drifted_editing = engine.editing_limits.clone();
            drifted_editing.max_operations_per_transaction = drifted_editing
                .max_operations_per_transaction
                .checked_sub(1)
                .expect("fixture editing limit is positive");
            let drifted_max_length = match engine.max_length {
                Some(_) => None,
                None => Some(1),
            };
            let drifted_document_revision = engine
                .revision
                .checked_add(1)
                .expect("fixture document revision can advance");
            let drifted_state_revision = engine
                .state_revision
                .checked_add(1)
                .expect("fixture state revision can advance");
            let drifted_schema = format!("{}!", engine.schema_fingerprint);

            let error = match case {
                "request" => context
                    .authority(
                        crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                            request_id: 65_251,
                            installed: state,
                            txn: &txn,
                            fragment: &fragment,
                            fragment_name: &engine.fragment_name,
                            schema_fingerprint: &engine.schema_fingerprint,
                            resource_limits: &engine.resource_limits,
                            editing_limits: &engine.editing_limits,
                            max_length: engine.max_length,
                            document_revision: engine.revision,
                            state_revision: engine.state_revision,
                            yrs_state_epoch: engine.yrs_state_epoch,
                        },
                    )
                    .err(),
                "store" => context
                    .authority(
                        crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                            request_id: 65_250,
                            installed: state,
                            txn: &foreign_txn,
                            fragment: &foreign_fragment,
                            fragment_name: &engine.fragment_name,
                            schema_fingerprint: &engine.schema_fingerprint,
                            resource_limits: &engine.resource_limits,
                            editing_limits: &engine.editing_limits,
                            max_length: engine.max_length,
                            document_revision: engine.revision,
                            state_revision: engine.state_revision,
                            yrs_state_epoch: engine.yrs_state_epoch,
                        },
                    )
                    .err(),
                "fragment" => context
                    .authority(
                        crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                            request_id: 65_250,
                            installed: state,
                            txn: &txn,
                            fragment: &fragment,
                            fragment_name: "foreign-fragment",
                            schema_fingerprint: &engine.schema_fingerprint,
                            resource_limits: &engine.resource_limits,
                            editing_limits: &engine.editing_limits,
                            max_length: engine.max_length,
                            document_revision: engine.revision,
                            state_revision: engine.state_revision,
                            yrs_state_epoch: engine.yrs_state_epoch,
                        },
                    )
                    .err(),
                "schema" => context
                    .authority(
                        crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                            request_id: 65_250,
                            installed: state,
                            txn: &txn,
                            fragment: &fragment,
                            fragment_name: &engine.fragment_name,
                            schema_fingerprint: &drifted_schema,
                            resource_limits: &engine.resource_limits,
                            editing_limits: &engine.editing_limits,
                            max_length: engine.max_length,
                            document_revision: engine.revision,
                            state_revision: engine.state_revision,
                            yrs_state_epoch: engine.yrs_state_epoch,
                        },
                    )
                    .err(),
                "resource_limits" => context
                    .authority(
                        crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                            request_id: 65_250,
                            installed: state,
                            txn: &txn,
                            fragment: &fragment,
                            fragment_name: &engine.fragment_name,
                            schema_fingerprint: &engine.schema_fingerprint,
                            resource_limits: &drifted_resources,
                            editing_limits: &engine.editing_limits,
                            max_length: engine.max_length,
                            document_revision: engine.revision,
                            state_revision: engine.state_revision,
                            yrs_state_epoch: engine.yrs_state_epoch,
                        },
                    )
                    .err(),
                "editing_limits" => context
                    .authority(
                        crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                            request_id: 65_250,
                            installed: state,
                            txn: &txn,
                            fragment: &fragment,
                            fragment_name: &engine.fragment_name,
                            schema_fingerprint: &engine.schema_fingerprint,
                            resource_limits: &engine.resource_limits,
                            editing_limits: &drifted_editing,
                            max_length: engine.max_length,
                            document_revision: engine.revision,
                            state_revision: engine.state_revision,
                            yrs_state_epoch: engine.yrs_state_epoch,
                        },
                    )
                    .err(),
                "max_length" => context
                    .authority(
                        crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                            request_id: 65_250,
                            installed: state,
                            txn: &txn,
                            fragment: &fragment,
                            fragment_name: &engine.fragment_name,
                            schema_fingerprint: &engine.schema_fingerprint,
                            resource_limits: &engine.resource_limits,
                            editing_limits: &engine.editing_limits,
                            max_length: drifted_max_length,
                            document_revision: engine.revision,
                            state_revision: engine.state_revision,
                            yrs_state_epoch: engine.yrs_state_epoch,
                        },
                    )
                    .err(),
                "document_revision" => context
                    .authority(
                        crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                            request_id: 65_250,
                            installed: state,
                            txn: &txn,
                            fragment: &fragment,
                            fragment_name: &engine.fragment_name,
                            schema_fingerprint: &engine.schema_fingerprint,
                            resource_limits: &engine.resource_limits,
                            editing_limits: &engine.editing_limits,
                            max_length: engine.max_length,
                            document_revision: drifted_document_revision,
                            state_revision: engine.state_revision,
                            yrs_state_epoch: engine.yrs_state_epoch,
                        },
                    )
                    .err(),
                "state_revision" => context
                    .authority(
                        crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                            request_id: 65_250,
                            installed: state,
                            txn: &txn,
                            fragment: &fragment,
                            fragment_name: &engine.fragment_name,
                            schema_fingerprint: &engine.schema_fingerprint,
                            resource_limits: &engine.resource_limits,
                            editing_limits: &engine.editing_limits,
                            max_length: engine.max_length,
                            document_revision: engine.revision,
                            state_revision: drifted_state_revision,
                            yrs_state_epoch: engine.yrs_state_epoch,
                        },
                    )
                    .err(),
                "epoch" => context
                    .authority(
                        crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                            request_id: 65_250,
                            installed: state,
                            txn: &txn,
                            fragment: &fragment,
                            fragment_name: &engine.fragment_name,
                            schema_fingerprint: &engine.schema_fingerprint,
                            resource_limits: &engine.resource_limits,
                            editing_limits: &engine.editing_limits,
                            max_length: engine.max_length,
                            document_revision: engine.revision,
                            state_revision: engine.state_revision,
                            yrs_state_epoch: engine.yrs_state_epoch.saturating_add(1),
                        },
                    )
                    .err(),
                "identity" => context
                    .authority(
                        crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                            request_id: 65_250,
                            installed: state,
                            txn: &txn,
                            fragment: &fragment,
                            fragment_name: &engine.fragment_name,
                            schema_fingerprint: &engine.schema_fingerprint,
                            resource_limits: &engine.resource_limits,
                            editing_limits: &engine.editing_limits,
                            max_length: engine.max_length,
                            document_revision: engine.revision,
                            state_revision: engine.state_revision,
                            yrs_state_epoch: engine.yrs_state_epoch,
                        },
                    )
                    .err(),
                _ => unreachable!(),
            }
            .expect("drifted live context must not mint an authority");
            assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{case}");
            drop(foreign_txn);
            drop(txn);
            assert_eq!(atomic_audit(&engine), before, "{case}");
            assert!(Arc::ptr_eq(
                &installed,
                &engine.derived_state.as_ref().unwrap().mutation_lookup_seed,
            ));
            let counts = take_prepared_admission_counts_for_test();
            assert_eq!(counts.staged_seed_preparations, 1, "{case}");
            assert_eq!(counts.installed_base_seed_publications, 0, "{case}");
        }
    }

    #[test]
    fn generic_typed_compilation_uses_staged_authority_without_publishing_base_seed() {
        use crate::yrs_engine::observability::{
            reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
        };

        let mut engine = import_document_with_unavailable_lookup_seed();
        let mut public = import_document_with_unavailable_lookup_seed();
        let mut public_rich = import_document_with_unavailable_lookup_seed();
        let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
        reset_prepared_admission_counts_for_test();
        let transaction = insert_transaction(&engine, 65_225);
        let (commit, result) = engine
            .apply_typed_transaction_with_staged_context(
                transaction,
                false,
                &mut OutboundUpdateSink::detached(),
            )
            .unwrap();
        assert!(result.is_none());
        let counts = take_prepared_admission_counts_for_test();
        let authority_counts = take_compiled_commit_authority_counts_for_test();
        assert_eq!(counts.staged_seed_preparations, 1);
        assert_eq!(counts.installed_base_seed_publications, 0);
        assert_eq!(authority_counts, (1, 1));
        assert!(!Arc::ptr_eq(
            &installed,
            &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
        ));
        assert!(engine
            .derived_state
            .as_ref()
            .unwrap()
            .mutation_lookup_seed
            .is_ready_for_test());

        reset_prepared_admission_counts_for_test();
        let public_commit = public
            .apply_typed_transaction(insert_transaction(&public, 65_225))
            .unwrap();
        let public_counts = take_prepared_admission_counts_for_test();
        assert_eq!(public_counts.staged_seed_preparations, 1);
        assert_eq!(public_counts.installed_base_seed_publications, 0);
        assert!(public
            .derived_state
            .as_ref()
            .unwrap()
            .mutation_lookup_seed
            .is_ready_for_test());
        assert_eq!(commit, public_commit);
        assert_eq!(engine.document_json(), public.document_json());
        assert_eq!(engine.resolved_selection(), public.resolved_selection());
        assert_eq!(engine.stored_marks(), public.stored_marks());
        assert_eq!(engine.can_undo(), public.can_undo());
        assert_eq!(engine.can_redo(), public.can_redo());

        reset_prepared_admission_counts_for_test();
        let rich_result = public_rich
            .apply_typed_transaction_with_result(insert_transaction(&public_rich, 65_225))
            .unwrap();
        assert!(rich_result.changed);
        let rich_counts = take_prepared_admission_counts_for_test();
        assert_eq!(rich_counts.staged_seed_preparations, 1);
        assert_eq!(rich_counts.installed_base_seed_publications, 0);
        assert!(public_rich
            .derived_state
            .as_ref()
            .unwrap()
            .mutation_lookup_seed
            .is_ready_for_test());
    }

    #[test]
    fn staged_generic_compiler_semantic_failure_is_prewrite_and_atomic() {
        use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};
        use crate::yrs_engine::observability::{
            reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
        };

        let mut engine = import_document_with_unavailable_lookup_seed();
        let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
        let before = atomic_audit(&engine);
        reset_prepared_admission_counts_for_test();
        set_atomic_failpoint_for_test(Some(AtomicFailpoint::SemanticCompilation));
        let transaction = insert_transaction(&engine, 65_226);
        let error = engine
            .apply_typed_transaction_with_staged_context(
                transaction,
                false,
                &mut OutboundUpdateSink::detached(),
            )
            .unwrap_err();
        set_atomic_failpoint_for_test(None);

        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
        assert_eq!(atomic_audit(&engine), before);
        assert!(Arc::ptr_eq(
            &installed,
            &engine.derived_state.as_ref().unwrap().mutation_lookup_seed,
        ));
        let counts = take_prepared_admission_counts_for_test();
        assert_eq!(counts.staged_seed_preparations, 1);
        assert_eq!(counts.installed_base_seed_publications, 0);
    }

    #[test]
    fn staged_generic_lookup_transition_failure_is_prewrite_and_atomic() {
        use crate::yrs_engine::observability::{
            reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
        };

        let mut engine = import_document_with_unavailable_lookup_seed();
        let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
        let before = atomic_audit(&engine);
        reset_prepared_admission_counts_for_test();
        set_compiled_commit_stage_failpoint_for_test(Some(
            CompiledCommitPreparationStage::LookupTransition,
        ));
        let transaction = insert_transaction(&engine, 65_227);
        let error = engine
            .apply_typed_transaction_with_staged_context(
                transaction,
                false,
                &mut OutboundUpdateSink::detached(),
            )
            .unwrap_err();
        set_compiled_commit_stage_failpoint_for_test(None);

        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
        assert_eq!(atomic_audit(&engine), before);
        assert!(Arc::ptr_eq(
            &installed,
            &engine.derived_state.as_ref().unwrap().mutation_lookup_seed,
        ));
        let counts = take_prepared_admission_counts_for_test();
        assert_eq!(counts.staged_seed_preparations, 1);
        assert_eq!(counts.installed_base_seed_publications, 0);
    }

    #[test]
    fn history_candidate_swap_prepares_ready_candidate_seed_without_compiled_transaction() {
        use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};
        use crate::yrs_engine::observability::{
            reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
        };
        use crate::yrs_engine::TransactionCommit;

        let mut engine = import_document_with_unavailable_lookup_seed();
        let mut public = import_document_with_unavailable_lookup_seed();
        engine
            .apply_command(65_226, TypedCommand::InsertText { text: "x".into() })
            .unwrap()
            .unwrap();
        public
            .apply_command(65_226, TypedCommand::InsertText { text: "x".into() })
            .unwrap()
            .unwrap();
        force_lookup_seed_unavailable(&mut engine);
        force_lookup_seed_unavailable(&mut public);
        let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
        reset_prepared_admission_counts_for_test();
        set_atomic_failpoint_for_test(Some(AtomicFailpoint::SemanticCompilation));
        let result =
            engine.apply_history_pop(65_227, true, true, &mut OutboundUpdateSink::detached());
        let compiler_failpoint = crate::yrs_engine::compiler::check_atomic_failpoint(
            65_227,
            AtomicFailpoint::SemanticCompilation,
        );
        set_atomic_failpoint_for_test(None);
        let (commit, result) = result.unwrap().unwrap();
        let result = result.unwrap();
        let compiler_error = compiler_failpoint.unwrap_err();
        assert_eq!(compiler_error.code, "ENGINE_INVARIANT_FAILED");
        let counts = take_prepared_admission_counts_for_test();
        assert_eq!(counts.staged_seed_preparations, 1);
        assert_eq!(counts.installed_base_seed_publications, 0);
        assert!(!Arc::ptr_eq(
            &installed,
            &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
        ));
        assert!(engine
            .derived_state
            .as_ref()
            .unwrap()
            .mutation_lookup_seed
            .is_ready_for_test());
        let state = engine.derived_state.as_ref().unwrap();
        let txn = engine.doc.transact();
        let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
        assert!(state
            .mutation_lookup_seed
            .matches_canonical_artifact(&state.canonical_artifact));
        assert!(state.mutation_lookup_seed.matches(
            &txn,
            &fragment,
            &state.document,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            &engine.schema_fingerprint,
            engine.yrs_state_epoch,
            engine.revision,
        ));
        drop(txn);
        assert_eq!(
            commit,
            TransactionCommit {
                request_id: result.request_id,
                changed: result.changed,
                document_revision: result.document_revision,
                state_revision: result.state_revision,
                origin: result.origin,
            }
        );

        let public_result = public.undo_with_result(65_227).unwrap().unwrap();
        assert_eq!(result, public_result);
        assert_eq!(engine.document_json(), public.document_json());
        assert_eq!(engine.resolved_selection(), public.resolved_selection());
        assert_eq!(engine.stored_marks(), public.stored_marks());
        assert_eq!(engine.can_undo(), public.can_undo());
        assert_eq!(engine.can_redo(), public.can_redo());
        assert_eq!(
            engine.history.replay_audit_for_test(),
            public.history.replay_audit_for_test()
        );
        assert_eq!(
            engine.history.retained_units(65_227).unwrap(),
            public.history.retained_units(65_227).unwrap()
        );
    }

    #[test]
    fn history_candidate_publication_failures_are_pre_swap_and_atomic() {
        use crate::yrs_engine::mutation::{
            set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
        };
        use crate::yrs_engine::observability::{
            reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
        };

        for (request_id, failpoint, stage) in [
            (
                65_228,
                LookupSeedHydrationFailpoint::CandidateBindingPublication,
                "candidateBindingPublication",
            ),
            (
                65_229,
                LookupSeedHydrationFailpoint::CandidateSeedPublication,
                "candidateSeedPublication",
            ),
        ] {
            let mut engine = import_document_with_unavailable_lookup_seed();
            engine
                .apply_command(
                    request_id - 1,
                    TypedCommand::InsertText { text: "x".into() },
                )
                .unwrap()
                .unwrap();
            force_lookup_seed_unavailable(&mut engine);
            let installed =
                Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
            let before = atomic_audit(&engine);
            reset_prepared_admission_counts_for_test();
            set_lookup_seed_hydration_failpoint_for_test(Some(failpoint));
            let error = engine
                .apply_history_pop(request_id, true, true, &mut OutboundUpdateSink::detached())
                .unwrap_err();
            set_lookup_seed_hydration_failpoint_for_test(None);

            assert_eq!(error.code, "OPERATION_RESOURCE_EXHAUSTED", "{stage}");
            assert_eq!(
                error.message.as_ref(),
                format!("mutation lookup seed allocation failed during {stage}"),
                "{stage}"
            );
            assert_eq!(
                error.details,
                Some(json!({ "field": "mutationLookupSeed" })),
                "{stage}"
            );
            assert_eq!(atomic_audit(&engine), before, "{stage}");
            assert!(Arc::ptr_eq(
                &installed,
                &engine.derived_state.as_ref().unwrap().mutation_lookup_seed,
            ));
            let counts = take_prepared_admission_counts_for_test();
            assert_eq!(counts.staged_seed_preparations, 0, "{stage}");
            assert_eq!(counts.installed_base_seed_publications, 0, "{stage}");
        }
    }

    fn task5_changed_remote_fixture() -> (YrsDocumentEngine, Vec<u8>) {
        let target = import_document_with_unavailable_lookup_seed();
        let base = target.encoded_state().unwrap();
        let mut source = YrsDocumentEngine::new(YrsEngineConfig {
            schema: tiptap_schema(),
            fragment_name: "prosemirror".into(),
            initialization_mode: crate::yrs_engine::InitializationMode::AwaitRemote,
            resource_limits: ResourceLimits::default(),
            editing_limits: crate::yrs_engine::EditingLimits::default(),
            max_length: None,
            scope: None,
        })
        .unwrap();
        source.apply_remote_update_v1(65_228, &base).unwrap();
        source
            .apply_command(65_229, TypedCommand::InsertText { text: "r".into() })
            .unwrap()
            .unwrap();
        let target_vector = target.doc.transact().state_vector();
        let delta = source
            .doc
            .transact()
            .encode_state_as_update_v1(&target_vector);
        (target, delta)
    }

    fn task5_candidate_publication_fixture() -> (
        YrsDocumentEngine,
        Doc,
        crate::model::Document,
        crate::yrs_engine::canonical::CanonicalArtifact,
        u64,
        u64,
    ) {
        let (engine, delta) = task5_changed_remote_fixture();
        let current_encoded = engine.encoded_state().unwrap();
        let candidate_doc =
            super::fresh_utf16_doc_excluding(&engine.durable_client_ids, engine.client_id());
        {
            let mut txn = candidate_doc.transact_mut();
            txn.apply_update(Update::decode_v1(&current_encoded).unwrap())
                .unwrap();
            txn.apply_update(Update::decode_v1(&delta).unwrap())
                .unwrap();
        }
        let (candidate_document, candidate_artifact) = {
            let txn = candidate_doc.transact();
            let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
            let json =
                crate::yrs_engine::YrsDocumentCodec::new(&engine.schema, &engine.resource_limits)
                    .read_json(&fragment, &txn)
                    .unwrap();
            let document =
                from_prosemirror_json(&json, &engine.schema, UnknownTypeMode::Preserve).unwrap();
            let artifact = engine.canonical_schema.derive(&document).unwrap();
            (document, artifact)
        };
        let next_revision = engine.revision.checked_add(1).unwrap();
        let next_epoch = engine.yrs_state_epoch.checked_add(1).unwrap();
        (
            engine,
            candidate_doc,
            candidate_document,
            candidate_artifact,
            next_revision,
            next_epoch,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare_history_candidate_capability_for_test<T: ReadTxn>(
        request_id: u64,
        txn: &T,
        fragment: &yrs::types::xml::XmlFragmentRef,
        schema: &crate::schema::Schema,
        source_document: &crate::model::Document,
        canonical_artifact: &crate::yrs_engine::canonical::CanonicalArtifact,
        resource_limits: &ResourceLimits,
        editing_limits: &crate::yrs_engine::EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
        yrs_state_epoch: u64,
        document_revision: u64,
    ) -> crate::yrs_engine::derived_state::HistoryMutationLookupCapability {
        let (json, admission) =
            crate::yrs_engine::derived_state::prepare_history_candidate_read_for_test(
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
            .unwrap()
            .into_parts();
        assert_eq!(&json, canonical_artifact.value());
        admission
            .expect("exact candidate read must create one consuming admission")
            .mint_capability_for_test(request_id, txn, fragment)
            .unwrap()
    }

    #[allow(clippy::too_many_arguments)]
    fn publish_history_candidate_seed_for_test<T: ReadTxn>(
        capability: crate::yrs_engine::derived_state::HistoryMutationLookupCapability,
        request_id: u64,
        txn: &T,
        fragment: &yrs::types::xml::XmlFragmentRef,
        schema: &crate::schema::Schema,
        source_document: &crate::model::Document,
        canonical_artifact: &crate::yrs_engine::canonical::CanonicalArtifact,
        resource_limits: &ResourceLimits,
        editing_limits: &crate::yrs_engine::EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
        yrs_state_epoch: u64,
        document_revision: u64,
    ) -> crate::yrs_engine::OperationResult<Arc<crate::yrs_engine::mutation::MutationLookupSeed>>
    {
        capability.prepare_candidate_publication(
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

    #[test]
    fn candidate_seed_publication_is_ready_and_bound_only_to_its_candidate_store() {
        use crate::yrs_engine::observability::{
            reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
        };

        let (engine, delta) = task5_changed_remote_fixture();
        let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
        let current_encoded = engine.encoded_state().unwrap();
        let candidate_doc =
            super::fresh_utf16_doc_excluding(&engine.durable_client_ids, engine.client_id());
        {
            let mut txn = candidate_doc.transact_mut();
            txn.apply_update(Update::decode_v1(&current_encoded).unwrap())
                .unwrap();
            txn.apply_update(Update::decode_v1(&delta).unwrap())
                .unwrap();
        }
        let (candidate_document, candidate_artifact, next_revision, next_epoch) = {
            let txn = candidate_doc.transact();
            let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
            let json =
                crate::yrs_engine::YrsDocumentCodec::new(&engine.schema, &engine.resource_limits)
                    .read_json(&fragment, &txn)
                    .unwrap();
            let document =
                from_prosemirror_json(&json, &engine.schema, UnknownTypeMode::Preserve).unwrap();
            DocumentValidator::validate(&document, &engine.schema, &engine.resource_limits)
                .unwrap();
            let artifact = engine.canonical_schema.derive(&document).unwrap();
            (
                document,
                artifact,
                engine.revision.checked_add(1).unwrap(),
                engine.yrs_state_epoch.checked_add(1).unwrap(),
            )
        };

        reset_prepared_admission_counts_for_test();
        let candidate_seed = {
            let txn = candidate_doc.transact();
            let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
            prepare_history_candidate_capability_for_test(
                65_233,
                &txn,
                &fragment,
                &engine.schema,
                &candidate_document,
                &candidate_artifact,
                &engine.resource_limits,
                &engine.editing_limits,
                engine.max_length,
                &engine.schema_fingerprint,
                next_epoch,
                next_revision,
            )
            .prepare_candidate_publication(
                65_233,
                &txn,
                &fragment,
                &engine.schema,
                &candidate_document,
                &candidate_artifact,
                &engine.resource_limits,
                &engine.editing_limits,
                engine.max_length,
                &engine.schema_fingerprint,
                next_epoch,
                next_revision,
            )
            .unwrap()
        };
        let counts = take_prepared_admission_counts_for_test();

        let candidate_txn = candidate_doc.transact();
        let candidate_fragment = candidate_txn
            .get_xml_fragment(engine.fragment_name.as_str())
            .unwrap();
        assert!(candidate_seed.is_ready_for_test());
        assert!(candidate_seed.matches_canonical_artifact(&candidate_artifact));
        assert!(candidate_seed.matches(
            &candidate_txn,
            &candidate_fragment,
            &candidate_document,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            &engine.schema_fingerprint,
            next_epoch,
            next_revision,
        ));
        let live_txn = engine.doc.transact();
        let live_fragment = live_txn
            .get_xml_fragment(engine.fragment_name.as_str())
            .unwrap();
        assert!(!candidate_seed.matches(
            &live_txn,
            &live_fragment,
            &candidate_document,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            &engine.schema_fingerprint,
            next_epoch,
            next_revision,
        ));
        assert!(Arc::ptr_eq(
            &installed,
            &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
        ));
        assert!(installed.is_unavailable_for_test());
        assert_eq!(counts.staged_seed_preparations, 1);
        assert_eq!(counts.installed_base_seed_publications, 0);
    }

    #[test]
    fn consumed_history_capability_cannot_be_replayed_through_a_general_seed_clone() {
        use crate::yrs_engine::mutation::{
            set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
        };
        use crate::yrs_engine::observability::{
            reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
        };

        let (
            engine,
            candidate_doc,
            candidate_document,
            candidate_artifact,
            next_revision,
            next_epoch,
        ) = task5_candidate_publication_fixture();
        let before = atomic_audit(&engine);
        let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
        let txn = candidate_doc.transact();
        let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
        let capability = prepare_history_candidate_capability_for_test(
            65_244,
            &txn,
            &fragment,
            &engine.schema,
            &candidate_document,
            &candidate_artifact,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            &engine.schema_fingerprint,
            next_epoch,
            next_revision,
        );
        let general_seed = capability
            .into_unavailable_seed_for_test(65_244)
            .expect("consuming conversion must publish one unavailable general seed");

        reset_prepared_admission_counts_for_test();
        set_lookup_seed_hydration_failpoint_for_test(Some(
            LookupSeedHydrationFailpoint::BindingPublication,
        ));
        let result = general_seed.as_ref().clone().prepare_candidate_publication(
            65_245,
            &txn,
            &fragment,
            &engine.schema,
            &candidate_document,
            &candidate_artifact,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            &engine.schema_fingerprint,
            next_epoch,
            next_revision,
        );
        set_lookup_seed_hydration_failpoint_for_test(None);

        let error = result.expect_err("a general seed clone must not retain the one-shot seal");
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
        let counts = take_prepared_admission_counts_for_test();
        assert_eq!(counts.staged_seed_preparations, 0);
        assert_eq!(counts.installed_base_seed_publications, 0);
        assert_eq!(atomic_audit(&engine), before);
        assert!(Arc::ptr_eq(
            &installed,
            &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
        ));
    }

    #[test]
    fn history_capability_rejects_request_relabeling_before_publication_probes() {
        use crate::yrs_engine::mutation::{
            set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
        };
        use crate::yrs_engine::observability::{
            reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
        };

        for (publish_candidate, failpoint) in [
            (true, LookupSeedHydrationFailpoint::BindingPublication),
            (false, LookupSeedHydrationFailpoint::SeedPublication),
        ] {
            let (
                engine,
                candidate_doc,
                candidate_document,
                candidate_artifact,
                next_revision,
                next_epoch,
            ) = task5_candidate_publication_fixture();
            let before = atomic_audit(&engine);
            let installed =
                Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
            let txn = candidate_doc.transact();
            let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
            let capability = prepare_history_candidate_capability_for_test(
                65_246,
                &txn,
                &fragment,
                &engine.schema,
                &candidate_document,
                &candidate_artifact,
                &engine.resource_limits,
                &engine.editing_limits,
                engine.max_length,
                &engine.schema_fingerprint,
                next_epoch,
                next_revision,
            );

            reset_prepared_admission_counts_for_test();
            set_lookup_seed_hydration_failpoint_for_test(Some(failpoint));
            let result = if publish_candidate {
                capability.prepare_candidate_publication(
                    65_247,
                    &txn,
                    &fragment,
                    &engine.schema,
                    &candidate_document,
                    &candidate_artifact,
                    &engine.resource_limits,
                    &engine.editing_limits,
                    engine.max_length,
                    &engine.schema_fingerprint,
                    next_epoch,
                    next_revision,
                )
            } else {
                capability.into_unavailable_seed_for_test(65_247)
            };
            set_lookup_seed_hydration_failpoint_for_test(None);

            let error = result.expect_err("a one-shot history request must not be relabeled");
            assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
            assert_eq!(error.request_id, 65_247);
            let counts = take_prepared_admission_counts_for_test();
            assert_eq!(counts.staged_seed_preparations, 0);
            assert_eq!(counts.installed_base_seed_publications, 0);
            assert_eq!(atomic_audit(&engine), before);
            assert!(Arc::ptr_eq(
                &installed,
                &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
            ));
        }
    }

    #[test]
    fn history_candidate_seed_publication_rejects_contradictory_claims_before_failpoints() {
        use crate::schema::presets::prosemirror_schema;
        use crate::yrs_engine::mutation::{
            set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
        };
        use crate::yrs_engine::observability::{
            reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
        };

        #[derive(Clone, Copy, Debug)]
        enum Case {
            Document,
            CanonicalArtifact,
            CanonicalIdentity,
            Schema,
            SchemaFingerprint,
            ResourceLimits,
            EditingLimits,
            MaxLength,
            Store,
            Revision,
            Epoch,
            Fragment,
        }

        for case in [
            Case::Document,
            Case::CanonicalArtifact,
            Case::CanonicalIdentity,
            Case::Schema,
            Case::SchemaFingerprint,
            Case::ResourceLimits,
            Case::EditingLimits,
            Case::MaxLength,
            Case::Store,
            Case::Revision,
            Case::Epoch,
            Case::Fragment,
        ] {
            for failpoint in [
                LookupSeedHydrationFailpoint::BindingPublication,
                LookupSeedHydrationFailpoint::SeedPublication,
            ] {
                let (
                    engine,
                    candidate_doc,
                    candidate_document,
                    candidate_artifact,
                    next_revision,
                    next_epoch,
                ) = task5_candidate_publication_fixture();
                let before = atomic_audit(&engine);
                let installed =
                    Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
                let txn = candidate_doc.transact();
                let candidate_fragment =
                    txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
                let unavailable = prepare_history_candidate_capability_for_test(
                    65_236,
                    &txn,
                    &candidate_fragment,
                    &engine.schema,
                    &candidate_document,
                    &candidate_artifact,
                    &engine.resource_limits,
                    &engine.editing_limits,
                    engine.max_length,
                    &engine.schema_fingerprint,
                    next_epoch,
                    next_revision,
                );
                drop(txn);
                let wrong_fragment = candidate_doc.get_or_insert_xml_fragment("foreign");
                let txn = candidate_doc.transact();
                let candidate_fragment =
                    txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
                let wrong_document = engine.derived_state.as_ref().unwrap().document.clone();
                let wrong_artifact = engine
                    .derived_state
                    .as_ref()
                    .unwrap()
                    .canonical_artifact
                    .clone();
                let fresh_same_content_artifact =
                    engine.canonical_schema.derive(&candidate_document).unwrap();
                let wrong_schema = prosemirror_schema();
                let mut wrong_resource_limits = engine.resource_limits.clone();
                wrong_resource_limits.max_input_bytes =
                    wrong_resource_limits.max_input_bytes.saturating_add(1);
                let mut wrong_editing_limits = engine.editing_limits.clone();
                wrong_editing_limits.max_operations_per_transaction = wrong_editing_limits
                    .max_operations_per_transaction
                    .saturating_add(1);
                let wrong_max_length = match engine.max_length {
                    Some(_) => None,
                    None => Some(u32::MAX),
                };
                let foreign_doc = super::fresh_utf16_doc_excluding(
                    &engine.durable_client_ids,
                    engine.client_id(),
                );
                let foreign_store_fragment =
                    foreign_doc.get_or_insert_xml_fragment(engine.fragment_name.as_str());
                let foreign_txn = foreign_doc.transact();
                let source_document = if matches!(case, Case::Document) {
                    &wrong_document
                } else {
                    &candidate_document
                };
                let canonical_artifact = match case {
                    Case::CanonicalArtifact => &wrong_artifact,
                    Case::CanonicalIdentity => &fresh_same_content_artifact,
                    _ => &candidate_artifact,
                };
                let schema = if matches!(case, Case::Schema) {
                    &wrong_schema
                } else {
                    &engine.schema
                };
                let resource_limits = if matches!(case, Case::ResourceLimits) {
                    &wrong_resource_limits
                } else {
                    &engine.resource_limits
                };
                let editing_limits = if matches!(case, Case::EditingLimits) {
                    &wrong_editing_limits
                } else {
                    &engine.editing_limits
                };
                let max_length = if matches!(case, Case::MaxLength) {
                    wrong_max_length
                } else {
                    engine.max_length
                };
                let schema_fingerprint = if matches!(case, Case::SchemaFingerprint) {
                    "contradictory-schema-fingerprint"
                } else {
                    engine.schema_fingerprint.as_str()
                };
                let revision = if matches!(case, Case::Revision) {
                    next_revision.saturating_add(1)
                } else {
                    next_revision
                };
                let epoch = if matches!(case, Case::Epoch) {
                    next_epoch.saturating_add(1)
                } else {
                    next_epoch
                };
                let fragment = if matches!(case, Case::Fragment) {
                    &wrong_fragment
                } else if matches!(case, Case::Store) {
                    &foreign_store_fragment
                } else {
                    &candidate_fragment
                };
                reset_prepared_admission_counts_for_test();
                set_lookup_seed_hydration_failpoint_for_test(Some(failpoint));
                let publish_txn = if matches!(case, Case::Store) {
                    &foreign_txn
                } else {
                    &txn
                };
                let error = publish_history_candidate_seed_for_test(
                    unavailable,
                    65_236,
                    publish_txn,
                    fragment,
                    schema,
                    source_document,
                    canonical_artifact,
                    resource_limits,
                    editing_limits,
                    max_length,
                    schema_fingerprint,
                    epoch,
                    revision,
                )
                .expect_err("contradictory history candidate claims must reject before probes");
                set_lookup_seed_hydration_failpoint_for_test(None);
                assert_eq!(
                    error.code, "ENGINE_INVARIANT_FAILED",
                    "{case:?}/{failpoint:?}"
                );
                let counts = take_prepared_admission_counts_for_test();
                assert_eq!(counts.staged_seed_preparations, 0, "{case:?}/{failpoint:?}");
                assert_eq!(
                    counts.installed_base_seed_publications, 0,
                    "{case:?}/{failpoint:?}"
                );
                assert_eq!(atomic_audit(&engine), before, "{case:?}/{failpoint:?}");
                assert!(Arc::ptr_eq(
                    &installed,
                    &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
                ));
            }
        }
    }

    #[test]
    fn history_candidate_seed_publication_rejects_same_store_deletion_after_mint() {
        use crate::yrs_engine::mutation::{
            set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
        };
        use crate::yrs_engine::observability::{
            reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
        };
        use yrs::types::Text;

        for failpoint in [
            LookupSeedHydrationFailpoint::BindingPublication,
            LookupSeedHydrationFailpoint::SeedPublication,
        ] {
            let (
                engine,
                candidate_doc,
                candidate_document,
                candidate_artifact,
                next_revision,
                next_epoch,
            ) = task5_candidate_publication_fixture();
            let before = atomic_audit(&engine);
            let installed =
                Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
            let (unavailable, text) = {
                let txn = candidate_doc.transact();
                let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
                let unavailable = prepare_history_candidate_capability_for_test(
                    65_237,
                    &txn,
                    &fragment,
                    &engine.schema,
                    &candidate_document,
                    &candidate_artifact,
                    &engine.resource_limits,
                    &engine.editing_limits,
                    engine.max_length,
                    &engine.schema_fingerprint,
                    next_epoch,
                    next_revision,
                );
                let XmlOut::Element(paragraph) = fragment.get(&txn, 0).unwrap() else {
                    panic!("candidate paragraph missing")
                };
                let XmlOut::Text(text) = paragraph.get(&txn, 0).unwrap() else {
                    panic!("candidate text missing")
                };
                (unavailable, text)
            };
            {
                let mut txn = candidate_doc.transact_mut();
                text.remove_range(&mut txn, 0, 1);
            }
            let txn = candidate_doc.transact();
            let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
            reset_prepared_admission_counts_for_test();
            set_lookup_seed_hydration_failpoint_for_test(Some(failpoint));
            let error = publish_history_candidate_seed_for_test(
                unavailable,
                65_237,
                &txn,
                &fragment,
                &engine.schema,
                &candidate_document,
                &candidate_artifact,
                &engine.resource_limits,
                &engine.editing_limits,
                engine.max_length,
                &engine.schema_fingerprint,
                next_epoch,
                next_revision,
            )
            .expect_err("same-store deletion after mint must reject before publication probes");
            set_lookup_seed_hydration_failpoint_for_test(None);
            assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{failpoint:?}");
            let counts = take_prepared_admission_counts_for_test();
            assert_eq!(counts.staged_seed_preparations, 0, "{failpoint:?}");
            assert_eq!(counts.installed_base_seed_publications, 0, "{failpoint:?}");
            assert_eq!(atomic_audit(&engine), before, "{failpoint:?}");
            assert!(Arc::ptr_eq(
                &installed,
                &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
            ));
        }
    }

    #[test]
    fn history_candidate_read_rejects_a_self_consistent_document_from_another_store_before_probes()
    {
        use crate::yrs_engine::mutation::{
            set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
        };
        use crate::yrs_engine::observability::{
            reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
        };

        let (
            engine,
            candidate_doc,
            _candidate_document,
            _candidate_artifact,
            next_revision,
            next_epoch,
        ) = task5_candidate_publication_fixture();
        let before = atomic_audit(&engine);
        let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
        let foreign_state = engine.derived_state.as_ref().unwrap();
        let txn = candidate_doc.transact();
        let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();

        reset_prepared_admission_counts_for_test();
        set_lookup_seed_hydration_failpoint_for_test(Some(
            LookupSeedHydrationFailpoint::BindingPublication,
        ));
        let result = crate::yrs_engine::derived_state::prepare_history_candidate_read_for_test(
            65_238,
            &txn,
            &fragment,
            &engine.schema,
            &foreign_state.document,
            &foreign_state.canonical_artifact,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            &engine.schema_fingerprint,
            next_epoch,
            next_revision,
        );
        set_lookup_seed_hydration_failpoint_for_test(None);

        let (_json, admission) = result
            .expect("exact codec read remains available for generic history fallback")
            .into_parts();
        assert!(
            admission.is_none(),
            "a self-consistent document/artifact from another store must not mint history proof"
        );
        let counts = take_prepared_admission_counts_for_test();
        assert_eq!(counts.staged_seed_preparations, 0);
        assert_eq!(counts.installed_base_seed_publications, 0);
        assert_eq!(atomic_audit(&engine), before);
        assert!(Arc::ptr_eq(
            &installed,
            &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
        ));
    }

    #[test]
    fn authoritative_store_rebind_rejects_a_foreign_candidate_store() {
        let (engine, delta) = task5_changed_remote_fixture();
        let current_encoded = engine.encoded_state().unwrap();
        let build_candidate = || {
            let doc =
                super::fresh_utf16_doc_excluding(&engine.durable_client_ids, engine.client_id());
            {
                let mut txn = doc.transact_mut();
                txn.apply_update(Update::decode_v1(&current_encoded).unwrap())
                    .unwrap();
                txn.apply_update(Update::decode_v1(&delta).unwrap())
                    .unwrap();
            }
            doc
        };
        let candidate_doc = build_candidate();
        let foreign_candidate_doc = build_candidate();
        let (candidate_document, candidate_artifact) = {
            let txn = candidate_doc.transact();
            let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
            let json =
                crate::yrs_engine::YrsDocumentCodec::new(&engine.schema, &engine.resource_limits)
                    .read_json(&fragment, &txn)
                    .unwrap();
            let document =
                from_prosemirror_json(&json, &engine.schema, UnknownTypeMode::Preserve).unwrap();
            let artifact = engine.canonical_schema.derive(&document).unwrap();
            (document, artifact)
        };
        let next_revision = engine.revision.checked_add(1).unwrap();
        let next_epoch = engine.yrs_state_epoch.checked_add(1).unwrap();
        let candidate_seed = {
            let txn = candidate_doc.transact();
            let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
            prepare_history_candidate_capability_for_test(
                65_234,
                &txn,
                &fragment,
                &engine.schema,
                &candidate_document,
                &candidate_artifact,
                &engine.resource_limits,
                &engine.editing_limits,
                engine.max_length,
                &engine.schema_fingerprint,
                next_epoch,
                next_revision,
            )
            .prepare_candidate_publication(
                65_234,
                &txn,
                &fragment,
                &engine.schema,
                &candidate_document,
                &candidate_artifact,
                &engine.resource_limits,
                &engine.editing_limits,
                engine.max_length,
                &engine.schema_fingerprint,
                next_epoch,
                next_revision,
            )
            .unwrap()
        };
        let foreign_txn = foreign_candidate_doc.transact();
        let foreign_fragment = foreign_txn
            .get_xml_fragment(engine.fragment_name.as_str())
            .unwrap();
        let live_txn = engine.doc.transact();
        let live_fragment = live_txn
            .get_xml_fragment(engine.fragment_name.as_str())
            .unwrap();

        let error = candidate_seed
            .prepare_authoritative_store_rebind(
                65_235,
                &foreign_txn,
                &foreign_fragment,
                &candidate_document,
                &candidate_artifact,
                &engine.resource_limits,
                &engine.editing_limits,
                engine.max_length,
                &engine.schema_fingerprint,
                next_epoch,
                next_revision,
                &live_txn,
                &live_fragment,
            )
            .expect_err("a foreign candidate store must not be relabeled as live authority");
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    }

    #[test]
    fn authoritative_store_rebind_rejects_a_foreign_live_fragment_before_probes() {
        use crate::yrs_engine::mutation::{
            set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
        };
        use crate::yrs_engine::observability::{
            reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
        };

        let (
            engine,
            candidate_doc,
            candidate_document,
            candidate_artifact,
            next_revision,
            next_epoch,
        ) = task5_candidate_publication_fixture();
        let candidate_seed = {
            let txn = candidate_doc.transact();
            let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
            prepare_history_candidate_capability_for_test(
                65_239,
                &txn,
                &fragment,
                &engine.schema,
                &candidate_document,
                &candidate_artifact,
                &engine.resource_limits,
                &engine.editing_limits,
                engine.max_length,
                &engine.schema_fingerprint,
                next_epoch,
                next_revision,
            )
            .prepare_candidate_publication(
                65_239,
                &txn,
                &fragment,
                &engine.schema,
                &candidate_document,
                &candidate_artifact,
                &engine.resource_limits,
                &engine.editing_limits,
                engine.max_length,
                &engine.schema_fingerprint,
                next_epoch,
                next_revision,
            )
            .unwrap()
        };
        let foreign_live_fragment = engine.doc.get_or_insert_xml_fragment("foreign-live");
        let before = atomic_audit(&engine);
        let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
        let candidate_txn = candidate_doc.transact();
        let candidate_fragment = candidate_txn
            .get_xml_fragment(engine.fragment_name.as_str())
            .unwrap();
        let live_txn = engine.doc.transact();

        reset_prepared_admission_counts_for_test();
        set_lookup_seed_hydration_failpoint_for_test(Some(
            LookupSeedHydrationFailpoint::BindingPublication,
        ));
        let result = candidate_seed.prepare_authoritative_store_rebind(
            65_240,
            &candidate_txn,
            &candidate_fragment,
            &candidate_document,
            &candidate_artifact,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            &engine.schema_fingerprint,
            next_epoch,
            next_revision,
            &live_txn,
            &foreign_live_fragment,
        );
        set_lookup_seed_hydration_failpoint_for_test(None);

        let error = result.expect_err("a foreign live fragment must reject before publication");
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
        let counts = take_prepared_admission_counts_for_test();
        assert_eq!(counts.staged_seed_preparations, 0);
        assert_eq!(counts.installed_base_seed_publications, 0);
        assert_eq!(atomic_audit(&engine), before);
        assert!(Arc::ptr_eq(
            &installed,
            &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
        ));
    }

    #[test]
    fn matching_history_seed_publications_reach_all_four_exact_failpoint_stages() {
        use crate::yrs_engine::mutation::{
            set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
        };
        use crate::yrs_engine::observability::{
            reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
        };

        for (failpoint, expected_stage) in [
            (
                LookupSeedHydrationFailpoint::BindingPublication,
                "candidateBindingPublication",
            ),
            (
                LookupSeedHydrationFailpoint::SeedPublication,
                "candidateSeedPublication",
            ),
        ] {
            let (
                engine,
                candidate_doc,
                candidate_document,
                candidate_artifact,
                next_revision,
                next_epoch,
            ) = task5_candidate_publication_fixture();
            let before = atomic_audit(&engine);
            let installed =
                Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
            let txn = candidate_doc.transact();
            let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
            let unavailable = prepare_history_candidate_capability_for_test(
                65_241,
                &txn,
                &fragment,
                &engine.schema,
                &candidate_document,
                &candidate_artifact,
                &engine.resource_limits,
                &engine.editing_limits,
                engine.max_length,
                &engine.schema_fingerprint,
                next_epoch,
                next_revision,
            );
            reset_prepared_admission_counts_for_test();
            set_lookup_seed_hydration_failpoint_for_test(Some(failpoint));
            let result = unavailable.prepare_candidate_publication(
                65_241,
                &txn,
                &fragment,
                &engine.schema,
                &candidate_document,
                &candidate_artifact,
                &engine.resource_limits,
                &engine.editing_limits,
                engine.max_length,
                &engine.schema_fingerprint,
                next_epoch,
                next_revision,
            );
            set_lookup_seed_hydration_failpoint_for_test(None);
            let error = result.expect_err("matching candidate must reach armed publication stage");
            assert_eq!(error.code, "OPERATION_RESOURCE_EXHAUSTED");
            assert_eq!(error.request_id, 65_241);
            assert_eq!(
                error.message.as_ref(),
                format!("mutation lookup seed allocation failed during {expected_stage}")
            );
            assert_eq!(
                error.details,
                Some(json!({ "field": "mutationLookupSeed" }))
            );
            let counts = take_prepared_admission_counts_for_test();
            assert_eq!(counts.staged_seed_preparations, 0);
            assert_eq!(counts.installed_base_seed_publications, 0);
            assert_eq!(atomic_audit(&engine), before);
            assert!(Arc::ptr_eq(
                &installed,
                &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
            ));
        }

        let (
            engine,
            candidate_doc,
            candidate_document,
            candidate_artifact,
            next_revision,
            next_epoch,
        ) = task5_candidate_publication_fixture();
        let candidate_seed = {
            let txn = candidate_doc.transact();
            let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
            prepare_history_candidate_capability_for_test(
                65_242,
                &txn,
                &fragment,
                &engine.schema,
                &candidate_document,
                &candidate_artifact,
                &engine.resource_limits,
                &engine.editing_limits,
                engine.max_length,
                &engine.schema_fingerprint,
                next_epoch,
                next_revision,
            )
            .prepare_candidate_publication(
                65_242,
                &txn,
                &fragment,
                &engine.schema,
                &candidate_document,
                &candidate_artifact,
                &engine.resource_limits,
                &engine.editing_limits,
                engine.max_length,
                &engine.schema_fingerprint,
                next_epoch,
                next_revision,
            )
            .unwrap()
        };
        let before = atomic_audit(&engine);
        let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
        let candidate_txn = candidate_doc.transact();
        let candidate_fragment = candidate_txn
            .get_xml_fragment(engine.fragment_name.as_str())
            .unwrap();
        let live_txn = engine.doc.transact();
        let live_fragment = live_txn
            .get_xml_fragment(engine.fragment_name.as_str())
            .unwrap();
        for (failpoint, expected_stage) in [
            (
                LookupSeedHydrationFailpoint::BindingPublication,
                "authoritativeStoreBindingPublication",
            ),
            (
                LookupSeedHydrationFailpoint::SeedPublication,
                "authoritativeStoreSeedPublication",
            ),
        ] {
            reset_prepared_admission_counts_for_test();
            set_lookup_seed_hydration_failpoint_for_test(Some(failpoint));
            let result = candidate_seed.prepare_authoritative_store_rebind(
                65_243,
                &candidate_txn,
                &candidate_fragment,
                &candidate_document,
                &candidate_artifact,
                &engine.resource_limits,
                &engine.editing_limits,
                engine.max_length,
                &engine.schema_fingerprint,
                next_epoch,
                next_revision,
                &live_txn,
                &live_fragment,
            );
            set_lookup_seed_hydration_failpoint_for_test(None);
            let error = result.expect_err("matching rebind must reach armed publication stage");
            assert_eq!(error.code, "OPERATION_RESOURCE_EXHAUSTED");
            assert_eq!(error.request_id, 65_243);
            assert_eq!(
                error.message.as_ref(),
                format!("mutation lookup seed allocation failed during {expected_stage}")
            );
            assert_eq!(
                error.details,
                Some(json!({ "field": "mutationLookupSeed" }))
            );
            let counts = take_prepared_admission_counts_for_test();
            assert_eq!(counts.staged_seed_preparations, 0);
            assert_eq!(counts.installed_base_seed_publications, 0);
            assert_eq!(atomic_audit(&engine), before);
            assert!(Arc::ptr_eq(
                &installed,
                &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
            ));
        }
    }

    #[test]
    fn changed_remote_candidate_installs_only_its_candidate_owned_seed() {
        use crate::yrs_engine::observability::{
            reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
        };

        let (mut engine, delta) = task5_changed_remote_fixture();
        let unavailable = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
        let unchanged = engine.encoded_state().unwrap();
        reset_prepared_admission_counts_for_test();
        assert!(
            !engine
                .apply_remote_update_v1(65_230, &unchanged)
                .unwrap()
                .changed
        );
        let unchanged_counts = take_prepared_admission_counts_for_test();
        assert_eq!(unchanged_counts.staged_seed_preparations, 0);
        assert_eq!(unchanged_counts.installed_base_seed_publications, 0);
        assert!(Arc::ptr_eq(
            &unavailable,
            &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
        ));

        reset_prepared_admission_counts_for_test();
        assert!(
            engine
                .apply_remote_update_v1(65_231, &delta)
                .unwrap()
                .changed
        );
        let changed_counts = take_prepared_admission_counts_for_test();
        assert_eq!(changed_counts.staged_seed_preparations, 1);
        assert_eq!(changed_counts.installed_base_seed_publications, 0);
        assert!(!Arc::ptr_eq(
            &unavailable,
            &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
        ));
        assert!(engine
            .derived_state
            .as_ref()
            .unwrap()
            .mutation_lookup_seed
            .is_ready_for_test());
    }

    #[test]
    fn remote_live_store_rebind_allocation_failure_is_prewrite_and_atomic() {
        use crate::yrs_engine::mutation::{
            set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
        };
        use crate::yrs_engine::observability::{
            reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
        };

        let (mut engine, delta) = task5_changed_remote_fixture();
        let before = atomic_audit(&engine);
        let quarantine_before = engine.quarantined_remote_update.clone();
        let ledger_before = engine.history.replay_ledger_allocation_audit_for_test();
        let unavailable = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
        reset_prepared_admission_counts_for_test();
        set_lookup_seed_hydration_failpoint_for_test(Some(
            LookupSeedHydrationFailpoint::SeedPublication,
        ));
        let result = engine.apply_remote_update_v1(65_232, &delta);
        set_lookup_seed_hydration_failpoint_for_test(None);
        let error = result.expect_err("live-store rebind allocation failure must reject");
        assert_eq!(error.code, "OPERATION_RESOURCE_EXHAUSTED");
        assert_eq!(
            error.message.as_ref(),
            "mutation lookup seed allocation failed during authoritativeStoreSeedPublication"
        );
        assert_eq!(
            error.details,
            Some(json!({ "field": "mutationLookupSeed" }))
        );
        let counts = take_prepared_admission_counts_for_test();
        assert_eq!(counts.staged_seed_preparations, 0);
        assert_eq!(counts.installed_base_seed_publications, 0);
        assert_eq!(atomic_audit(&engine), before);
        assert_eq!(engine.quarantined_remote_update, quarantine_before);
        assert_eq!(
            engine.history.replay_ledger_allocation_audit_for_test(),
            ledger_before
        );
        assert!(Arc::ptr_eq(
            &unavailable,
            &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
        ));
    }

    #[test]
    fn deferred_finalization_reuses_saved_evidence_without_revalidation() {
        use crate::yrs_engine::observability::{
            reset_full_pass_counts_for_test, reset_prepared_admission_counts_for_test,
            take_full_pass_counts_for_test, take_prepared_admission_counts_for_test,
        };

        reset_prepared_admission_counts_for_test();
        let (engine, deferred, mut context, transaction, expected_document) =
            deferred_finalization_fixture();
        engine.prepare_mutation_identity(&mut context).unwrap();
        reset_full_pass_counts_for_test();
        let prepared = engine
            .finalize_deferred_for_test(deferred, &context, &transaction, &expected_document)
            .unwrap();
        assert!(prepared.admits_expected_document(&expected_document));
        let passes = take_full_pass_counts_for_test();
        let admission = take_prepared_admission_counts_for_test();
        assert_eq!(passes.planner_simulations, 0);
        assert_eq!(passes.document_validations, 0);
        assert_eq!(passes.render_limit_tree_scans, 0);
        assert_eq!(passes.render_identity_scans, 0);
        assert_eq!(admission.deferred_capsules_created, 1);
        assert_eq!(admission.deferred_capsules_finalized, 1);
    }

    #[test]
    fn deferred_capsule_tamper_cases_reject_before_write() {
        for case in
            crate::yrs_engine::prepared_admission::DeferredCommandAdmission::tamper_cases_for_test()
        {
            let (engine, deferred, mut context, transaction, expected_document) =
                deferred_tamper_fixture(case);
            engine.prepare_mutation_identity(&mut context).unwrap();
            let before = atomic_audit(&engine);
            let error = engine
                .finalize_deferred_for_test(deferred, &context, &transaction, &expected_document)
                .expect_err(&format!("tampered deferred capsule must reject: {case}"));
            assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{case}");
            assert_eq!(atomic_audit(&engine), before, "{case}");
        }
    }

    #[test]
    fn deferred_same_summary_evidence_replacements_reject_without_identity_scans() {
        use crate::yrs_engine::observability::{
            reset_full_pass_counts_for_test, take_full_pass_counts_for_test,
        };

        for case in ["position", "render"] {
            let (engine, mut deferred, mut context, transaction, expected_document) =
                deferred_finalization_fixture();
            deferred.tamper_same_summary_evidence_for_test(case);
            engine.prepare_mutation_identity(&mut context).unwrap();
            let before = atomic_audit(&engine);
            reset_full_pass_counts_for_test();

            let error = engine
                .finalize_deferred_for_test(deferred, &context, &transaction, &expected_document)
                .unwrap_err();

            assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{case}");
            assert_eq!(atomic_audit(&engine), before, "{case}");
            let passes = take_full_pass_counts_for_test();
            assert_eq!(passes.position_map_clones, 0, "{case}");
            assert_eq!(passes.render_limit_tree_scans, 0, "{case}");
            assert_eq!(passes.render_identity_scans, 0, "{case}");
        }
    }

    #[test]
    fn deferred_shape_rejects_matching_transaction_position_tamper() {
        let (engine, mut deferred, mut context, mut transaction, expected_document) =
            deferred_finalization_fixture();
        deferred.tamper_matching_transaction_position_for_test(&mut transaction);
        engine.prepare_mutation_identity(&mut context).unwrap();
        let before = atomic_audit(&engine);

        let error = engine
            .finalize_deferred_for_test(deferred, &context, &transaction, &expected_document)
            .unwrap_err();

        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
        assert_eq!(atomic_audit(&engine), before);
    }

    #[test]
    fn deferred_finalization_preserves_warmed_candidate_scalar_caches() {
        use crate::yrs_engine::observability::{
            reset_full_pass_counts_for_test, take_full_pass_counts_for_test,
        };

        let (engine, deferred, mut context, transaction, expected_document) =
            deferred_finalization_fixture();
        let (expected_len, expected_sha256) = deferred.warm_candidate_caches_for_test();
        engine.prepare_mutation_identity(&mut context).unwrap();
        reset_full_pass_counts_for_test();

        let prepared = engine
            .finalize_deferred_for_test(deferred, &context, &transaction, &expected_document)
            .unwrap();

        assert_eq!(prepared.canonical_artifact().serialized_len(), expected_len);
        assert_eq!(prepared.canonical_artifact().sha256(), expected_sha256);
        let passes = take_full_pass_counts_for_test();
        assert_eq!(passes.canonical_serializations, 0);
        assert_eq!(passes.canonical_hashes, 0);
    }

    #[test]
    fn deferred_finalization_rejects_mismatched_prefilled_candidate_caches() {
        use crate::yrs_engine::observability::{
            reset_full_pass_counts_for_test, take_full_pass_counts_for_test,
        };

        for case in ["length", "sha256"] {
            let (engine, mut deferred, mut context, transaction, expected_document) =
                deferred_finalization_fixture();
            let _ = deferred.warm_candidate_caches_for_test();
            deferred.tamper_candidate_cache_for_test(case);
            engine.prepare_mutation_identity(&mut context).unwrap();
            let before = atomic_audit(&engine);
            reset_full_pass_counts_for_test();

            let error = engine
                .finalize_deferred_for_test(deferred, &context, &transaction, &expected_document)
                .unwrap_err();

            assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{case}");
            assert_eq!(atomic_audit(&engine), before, "{case}");
            let passes = take_full_pass_counts_for_test();
            assert_eq!(passes.canonical_serializations, 0, "{case}");
            assert_eq!(passes.canonical_hashes, 0, "{case}");
        }
    }

    #[test]
    fn imported_commands_plan_not_applicable_and_stored_marks_before_hydration() {
        use crate::yrs_engine::mutation::{
            set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
        };
        use crate::yrs_engine::observability::{
            reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
        };

        let mut not_applicable = import_document_with_unavailable_lookup_seed();
        reset_prepared_admission_counts_for_test();
        set_lookup_seed_hydration_failpoint_for_test(Some(
            LookupSeedHydrationFailpoint::InitialReservation,
        ));
        let result = not_applicable
            .apply_command(65_130, TypedCommand::ToggleTaskItemChecked)
            .unwrap();
        set_lookup_seed_hydration_failpoint_for_test(None);
        assert!(result.is_none());
        let not_applicable_counts = take_prepared_admission_counts_for_test();
        assert_eq!(not_applicable_counts.staged_seed_preparations, 0);
        assert_eq!(not_applicable_counts.installed_base_seed_publications, 0);
        assert!(not_applicable
            .derived_state
            .as_ref()
            .unwrap()
            .mutation_lookup_seed
            .is_unavailable_for_test());

        let mut stored_mark = import_document_with_unavailable_lookup_seed();
        reset_prepared_admission_counts_for_test();
        set_lookup_seed_hydration_failpoint_for_test(Some(
            LookupSeedHydrationFailpoint::InitialReservation,
        ));
        let result = stored_mark
            .apply_command(
                65_131,
                TypedCommand::ToggleMark {
                    mark_type: "bold".into(),
                },
            )
            .unwrap();
        set_lookup_seed_hydration_failpoint_for_test(None);
        assert!(result.is_some());
        let stored_mark_counts = take_prepared_admission_counts_for_test();
        assert_eq!(stored_mark_counts.staged_seed_preparations, 0);
        assert_eq!(stored_mark_counts.installed_base_seed_publications, 0);
        assert_eq!(
            stored_mark
                .stored_marks()
                .unwrap()
                .iter()
                .map(Mark::mark_type)
                .collect::<Vec<_>>(),
            vec!["bold"]
        );
        assert!(stored_mark
            .derived_state
            .as_ref()
            .unwrap()
            .mutation_lookup_seed
            .is_unavailable_for_test());
    }

    #[test]
    fn immediate_import_local_input_local_api_and_structural_routes_hydrate_real_consumers() {
        let mut local_input = import_document_with_unavailable_lookup_seed();
        let mut transaction = insert_transaction(&local_input, 65_140);
        transaction.origin = TransactionOrigin::LocalInput;
        local_input.apply_typed_transaction(transaction).unwrap();
        assert!(local_input
            .derived_state
            .as_ref()
            .unwrap()
            .mutation_lookup_seed
            .is_ready_for_test());

        let mut local_api = import_document_with_unavailable_lookup_seed();
        local_api
            .apply_typed_transaction(insert_transaction(&local_api, 65_141))
            .unwrap();
        assert!(local_api
            .derived_state
            .as_ref()
            .unwrap()
            .mutation_lookup_seed
            .is_ready_for_test());

        let mut structural = import_document_with_unavailable_lookup_seed();
        crate::yrs_engine::mutation::reset_localized_lookup_counts_for_test();
        structural
            .apply_command(
                65_142,
                TypedCommand::WrapInList {
                    list_type: "bulletList".into(),
                    item_type: "listItem".into(),
                },
            )
            .unwrap()
            .expect("paragraph should wrap in a bullet list");
        assert_eq!(
            crate::yrs_engine::mutation::take_localized_lookup_counts_for_test(),
            (0, 0, 0),
            "the structural command must consume the staged seed without a live rebuild"
        );
        assert_eq!(
            structural.document_json().unwrap()["content"][0]["type"],
            "bulletList"
        );
    }

    #[test]
    fn immediate_import_noop_remote_candidate_does_not_hydrate_live_seed() {
        use crate::yrs_engine::mutation::{
            set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
        };

        let mut engine = import_document_with_unavailable_lookup_seed();
        let update = engine.encoded_state().unwrap();
        let unavailable = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
        set_lookup_seed_hydration_failpoint_for_test(Some(
            LookupSeedHydrationFailpoint::InitialReservation,
        ));

        let commit = engine.apply_remote_update_v1(65_143, &update).unwrap();

        set_lookup_seed_hydration_failpoint_for_test(None);
        assert!(!commit.changed);
        assert!(Arc::ptr_eq(
            &unavailable,
            &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
        ));
        assert!(engine
            .derived_state
            .as_ref()
            .unwrap()
            .mutation_lookup_seed
            .is_unavailable_for_test());

        let mut source = YrsDocumentEngine::new(YrsEngineConfig {
            schema: tiptap_schema(),
            fragment_name: "prosemirror".into(),
            initialization_mode: crate::yrs_engine::InitializationMode::AwaitRemote,
            resource_limits: ResourceLimits::default(),
            editing_limits: crate::yrs_engine::EditingLimits::default(),
            max_length: None,
            scope: None,
        })
        .unwrap();
        source.apply_remote_update_v1(65_144, &update).unwrap();
        source
            .apply_command(65_145, TypedCommand::InsertText { text: "r".into() })
            .unwrap()
            .unwrap();
        let target_vector = engine.doc.transact().state_vector();
        let delta = source
            .doc
            .transact()
            .encode_state_as_update_v1(&target_vector);

        let commit = engine.apply_remote_update_v1(65_146, &delta).unwrap();

        assert!(commit.changed);
        assert!(engine
            .derived_state
            .as_ref()
            .unwrap()
            .mutation_lookup_seed
            .is_ready_for_test());
    }

    #[test]
    fn prepare_mutation_context_does_not_publish_the_installed_seed() {
        let engine = import_document_with_unavailable_lookup_seed();
        let before = atomic_audit(&engine);
        let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
        crate::yrs_engine::observability::reset_prepared_admission_counts_for_test();
        let context = engine.prepare_mutation_lookup_seed(65_210).unwrap();
        assert!(context.lookup_seed().is_ready_for_test());
        assert!(Arc::ptr_eq(
            &installed,
            &engine.derived_state.as_ref().unwrap().mutation_lookup_seed,
        ));
        assert_eq!(atomic_audit(&engine), before);
        let counts = crate::yrs_engine::observability::take_prepared_admission_counts_for_test();
        assert_eq!(counts.staged_seed_preparations, 1);
        assert_eq!(counts.installed_base_seed_publications, 0);
    }

    #[test]
    fn prepared_mutation_identity_is_lazy_and_does_not_mutate_installed_caches() {
        let engine = import_document_with_unavailable_lookup_seed();
        let mut context = engine.prepare_mutation_lookup_seed(65_211).unwrap();
        crate::yrs_engine::observability::reset_prepared_admission_counts_for_test();
        assert!(context.materialized_identity().is_none());
        engine.prepare_mutation_identity(&mut context).unwrap();
        assert!(context.materialized_identity().is_some());
        assert_eq!(
            crate::yrs_engine::observability::take_prepared_admission_counts_for_test()
                .staged_identity_materializations,
            1,
        );
        assert!(!engine
            .derived_state
            .as_ref()
            .unwrap()
            .validation_certificate
            .canonical_fingerprint_materialized_for_test());
        assert!(!engine
            .derived_state
            .as_ref()
            .unwrap()
            .localized_text_index
            .as_ref()
            .unwrap()
            .canonical_fingerprint_materialized_for_test());
    }

    #[test]
    fn prepared_mutation_authority_rejects_request_mismatch_atomically() {
        let engine = import_document_with_unavailable_lookup_seed();
        let before = atomic_audit(&engine);
        let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
        crate::yrs_engine::observability::reset_prepared_admission_counts_for_test();
        let context = engine.prepare_mutation_lookup_seed(65_212).unwrap();
        let state = engine.derived_state.as_ref().unwrap();
        let txn = engine.doc.transact();
        let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();

        let error = match context.authority(
            crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                request_id: 65_213,
                installed: state,
                txn: &txn,
                fragment: &fragment,
                fragment_name: &engine.fragment_name,
                schema_fingerprint: &engine.schema_fingerprint,
                resource_limits: &engine.resource_limits,
                editing_limits: &engine.editing_limits,
                max_length: engine.max_length,
                document_revision: engine.revision,
                state_revision: engine.state_revision,
                yrs_state_epoch: engine.yrs_state_epoch,
            },
        ) {
            Ok(_) => panic!("a prepared context must not authorize another request"),
            Err(error) => error,
        };
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
        assert_eq!(error.request_id, 65_212);

        {
            let authority = context
                .authority(
                    crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                        request_id: 65_212,
                        installed: state,
                        txn: &txn,
                        fragment: &fragment,
                        fragment_name: &engine.fragment_name,
                        schema_fingerprint: &engine.schema_fingerprint,
                        resource_limits: &engine.resource_limits,
                        editing_limits: &engine.editing_limits,
                        max_length: engine.max_length,
                        document_revision: engine.revision,
                        state_revision: engine.state_revision,
                        yrs_state_epoch: engine.yrs_state_epoch,
                    },
                )
                .unwrap();
            assert!(authority.lookup_seed().is_ready_for_test());
        }
        drop(txn);

        assert!(Arc::ptr_eq(
            &installed,
            &engine.derived_state.as_ref().unwrap().mutation_lookup_seed,
        ));
        assert_eq!(atomic_audit(&engine), before);
        let counts = crate::yrs_engine::observability::take_prepared_admission_counts_for_test();
        assert_eq!(counts.staged_seed_preparations, 1);
        assert_eq!(counts.installed_base_seed_publications, 0);
    }

    #[test]
    fn lookup_seed_rejects_same_value_stale_canonical_artifact_identity() {
        let mut engine = import_document_with_unavailable_lookup_seed();
        engine.ensure_mutation_lookup_seed(65_108).unwrap();
        let state = engine.derived_state.as_ref().unwrap();
        let stale_seed = Arc::clone(&state.mutation_lookup_seed);
        assert!(stale_seed.matches_canonical_artifact(&state.canonical_artifact));

        let replacement = state
            .canonical_artifact
            .schema_context()
            .derive(&state.document)
            .unwrap();
        assert!(!replacement.ptr_eq(&state.canonical_artifact));
        engine.derived_state.as_mut().unwrap().canonical_artifact = replacement;
        assert!(!stale_seed.matches_canonical_artifact(
            &engine.derived_state.as_ref().unwrap().canonical_artifact
        ));

        crate::yrs_engine::mutation::reset_localized_lookup_counts_for_test();
        engine.ensure_mutation_lookup_seed(65_109).unwrap();
        assert_eq!(
            crate::yrs_engine::mutation::take_localized_lookup_counts_for_test().0,
            1
        );
        let state = engine.derived_state.as_ref().unwrap();
        assert!(state
            .mutation_lookup_seed
            .matches_canonical_artifact(&state.canonical_artifact));
    }

    #[test]
    fn unavailable_lookup_hydration_failure_is_atomic() {
        let mut engine = import_document_with_unavailable_lookup_seed();
        engine.fragment_name = "missing-after-import".into();
        let before = atomic_audit(&engine);
        let unavailable = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);

        let error = engine
            .apply_command(65_108, TypedCommand::InsertText { text: "x".into() })
            .unwrap_err();

        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
        assert!(Arc::ptr_eq(
            &unavailable,
            &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
        ));
        assert_eq!(atomic_audit(&engine), before);
    }

    #[test]
    fn unavailable_lookup_allocation_failpoints_are_resource_errors_and_atomic() {
        use crate::yrs_engine::mutation::{
            set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
        };

        for (index, failpoint) in [
            LookupSeedHydrationFailpoint::InitialReservation,
            LookupSeedHydrationFailpoint::MapGrowth,
            LookupSeedHydrationFailpoint::MapPublication,
            LookupSeedHydrationFailpoint::BindingPublication,
            LookupSeedHydrationFailpoint::SeedPublication,
        ]
        .into_iter()
        .enumerate()
        {
            let mut engine = import_document_with_unavailable_lookup_seed();
            assert!(engine.prepared_candidate_cache.take().is_some());
            let before = atomic_audit(&engine);
            let unavailable =
                Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
            set_lookup_seed_hydration_failpoint_for_test(Some(failpoint));

            let error = engine
                .apply_command(
                    65_120 + index as u64,
                    TypedCommand::InsertText { text: "x".into() },
                )
                .unwrap_err();

            set_lookup_seed_hydration_failpoint_for_test(None);
            assert_eq!(error.code, "OPERATION_RESOURCE_EXHAUSTED", "{failpoint:?}");
            assert_eq!(
                error.details,
                Some(json!({ "field": "mutationLookupSeed" })),
                "{failpoint:?}"
            );
            assert!(
                Arc::ptr_eq(
                    &unavailable,
                    &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
                ),
                "{failpoint:?}"
            );
            assert_eq!(atomic_audit(&engine), before, "{failpoint:?}");
        }
    }

    #[test]
    fn lookup_seed_hydration_does_not_reserve_growth_with_spare_capacity() {
        use crate::yrs_engine::mutation::{
            reset_lookup_seed_map_growth_attempts_for_test,
            take_lookup_seed_map_growth_attempts_for_test,
        };

        let mut engine = import_document_with_unavailable_lookup_seed();
        assert!(engine.prepared_candidate_cache.take().is_some());
        reset_lookup_seed_map_growth_attempts_for_test();
        engine
            .apply_command(65_126, TypedCommand::InsertText { text: "x".into() })
            .unwrap()
            .unwrap();

        assert_eq!(take_lookup_seed_map_growth_attempts_for_test(), 0);
    }

    #[test]
    fn engine_commands_reuse_the_proven_schema_context_without_recomputing_it() {
        use crate::yrs_engine::canonical::{
            reset_canonical_schema_context_count_for_test,
            take_canonical_schema_context_count_for_test,
        };

        let mut engine = transaction_engine();
        reset_canonical_schema_context_count_for_test();
        engine
            .apply_command(65_001, TypedCommand::InsertText { text: "x".into() })
            .unwrap();

        assert_eq!(take_canonical_schema_context_count_for_test(), 0);
    }

    #[test]
    fn collision_excluding_candidate_selection_retries_live_and_durable_ids() {
        let durable = HashSet::from([7_u64]);
        let mut ids = [5_u64, 7_u64, 11_u64].into_iter();
        let selected = fresh_utf16_doc_excluding_with(&durable, 5, || {
            Doc::with_options(Options {
                client_id: ClientID::new(ids.next().unwrap()),
                offset_kind: OffsetKind::Utf16,
                ..Options::default()
            })
        });

        assert_eq!(selected.client_id().get(), 11);
    }

    #[test]
    fn restored_and_local_candidates_cache_all_relevant_durable_clients() {
        let config = || crate::yrs_engine::YrsEngineConfig {
            schema: tiptap_schema(),
            fragment_name: "prosemirror".into(),
            initialization_mode: crate::yrs_engine::InitializationMode::LocalEmpty,
            resource_limits: ResourceLimits::default(),
            editing_limits: crate::yrs_engine::EditingLimits::default(),
            max_length: None,
            scope: Some(crate::yrs_engine::DocumentScope {
                document_id: "doc".into(),
                lineage_id: "lineage".into(),
            }),
        };
        let source = crate::yrs_engine::YrsDocumentEngine::new(config()).unwrap();
        let snapshot = source.export_snapshot().unwrap();
        let expected = Update::decode_v1(&snapshot.encoded_state)
            .unwrap()
            .state_vector()
            .iter()
            .map(|(client, _)| client.get())
            .collect::<HashSet<_>>();
        let mut target = crate::yrs_engine::YrsDocumentEngine::new(config()).unwrap();

        target.restore_snapshot(&snapshot).unwrap();
        assert_eq!(target.durable_client_ids, expected);

        target
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"local"}]}]}"#,
                crate::yrs_engine::TransactionOrigin::DocumentImport,
            )
            .unwrap();
        assert_eq!(
            target.durable_client_ids,
            HashSet::from([target.client_id()])
        );
    }

    #[test]
    fn revision_overflow_rejects_before_candidate_swap() {
        let mut engine =
            crate::yrs_engine::YrsDocumentEngine::new(crate::yrs_engine::YrsEngineConfig {
                schema: tiptap_schema(),
                fragment_name: "prosemirror".into(),
                initialization_mode: crate::yrs_engine::InitializationMode::LocalEmpty,
                resource_limits: ResourceLimits::default(),
                editing_limits: crate::yrs_engine::EditingLimits::default(),
                max_length: None,
                scope: None,
            })
            .unwrap();
        engine.revision = u64::MAX;
        engine.derived_state.as_mut().unwrap().document_revision = u64::MAX;
        let before_client = engine.client_id();
        let before_json = engine.document_json();
        let before_state = engine.encoded_state().unwrap();

        let error = engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"changed"}]}]}"#,
                crate::yrs_engine::TransactionOrigin::DocumentImport,
            )
            .unwrap_err();

        assert_eq!(error.code, "REVISION_OVERFLOW");
        assert_eq!(engine.revision(), u64::MAX);
        assert_eq!(engine.client_id(), before_client);
        assert_eq!(engine.document_json(), before_json);
        assert_eq!(engine.encoded_state().unwrap(), before_state);
    }

    #[test]
    fn candidate_state_revision_and_epoch_overflow_reject_before_swap() {
        for field in ["stateRevision", "yrsStateEpoch"] {
            let mut engine = transaction_engine();
            if field == "stateRevision" {
                engine.state_revision = u64::MAX;
                engine.derived_state.as_mut().unwrap().state_revision = u64::MAX;
            } else {
                engine.yrs_state_epoch = u64::MAX;
            }
            let before = atomic_audit(&engine);

            let error = engine
                .import_json(
                    r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"changed"}]}]}"#,
                    TransactionOrigin::DocumentImport,
                )
                .unwrap_err();

            assert_eq!(error.code, "REVISION_OVERFLOW", "{field}");
            assert_eq!(error.details, Some(json!({ "field": field })), "{field}");
            assert_eq!(atomic_audit(&engine), before, "{field}");
        }
    }

    #[test]
    fn identical_selection_is_no_op_even_when_state_revision_is_max() {
        let mut engine = transaction_engine();
        engine.state_revision = u64::MAX;
        if let Some(state) = &mut engine.derived_state {
            state.state_revision = u64::MAX;
        }
        let before = atomic_audit(&engine);
        let transaction = TypedTransaction {
            request_id: 90_001,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent: SelectionIntent::Set(crate::yrs_engine::SelectionInput::Text {
                anchor: RevisionedPosition {
                    offset: 0,
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::Before,
                },
                head: RevisionedPosition {
                    offset: 0,
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::Before,
                },
            }),
            history_policy: HistoryPolicy::Skip,
        };
        let commit = engine.apply_typed_transaction(transaction).unwrap();
        assert!(!commit.changed);
        assert_eq!(atomic_audit(&engine), before);
    }

    #[test]
    fn snapshot_export_envelope_budget_has_exact_and_over_boundaries_without_mutation() {
        let mut engine =
            crate::yrs_engine::YrsDocumentEngine::new(crate::yrs_engine::YrsEngineConfig {
                schema: tiptap_schema(),
                fragment_name: "prosemirror".into(),
                initialization_mode: crate::yrs_engine::InitializationMode::LocalEmpty,
                resource_limits: ResourceLimits::default(),
                editing_limits: crate::yrs_engine::EditingLimits::default(),
                max_length: None,
                scope: Some(crate::yrs_engine::DocumentScope {
                    document_id: "doc".into(),
                    lineage_id: "lineage".into(),
                }),
            })
            .unwrap();
        let state = engine.encoded_state().unwrap();
        let metadata_bytes =
            "doc".len() + "lineage".len() + "prosemirror".len() + engine.schema_fingerprint().len();
        engine.resource_limits.max_input_bytes = metadata_bytes;
        engine.resource_limits.max_encoded_state_bytes = state.len();
        assert!(engine.export_snapshot().is_ok());

        let before_revision = engine.revision();
        let before_client = engine.client_id();
        let before_json = engine.document_json();
        engine.resource_limits.max_input_bytes = metadata_bytes - 1;
        let error = engine.export_snapshot().unwrap_err();

        assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
        assert_eq!(
            error.details,
            Some(serde_json::json!({"phase": "snapshotExport"}))
        );
        assert_eq!(engine.revision(), before_revision);
        assert_eq!(engine.client_id(), before_client);
        assert_eq!(engine.document_json(), before_json);
        assert_eq!(engine.encoded_state().unwrap(), state);
    }

    #[test]
    fn typed_transaction_rejects_every_revision_or_epoch_overflow_before_mutation() {
        for field in ["documentRevision", "stateRevision", "yrsStateEpoch"] {
            let mut engine = transaction_engine();
            match field {
                "documentRevision" => {
                    engine.revision = u64::MAX;
                    engine.derived_state.as_mut().unwrap().document_revision = u64::MAX;
                }
                "stateRevision" => {
                    engine.state_revision = u64::MAX;
                    engine.derived_state.as_mut().unwrap().state_revision = u64::MAX;
                }
                "yrsStateEpoch" => engine.yrs_state_epoch = u64::MAX,
                _ => unreachable!(),
            }
            let transaction = insert_transaction(&engine, 71);
            let before = atomic_audit(&engine);

            let error = engine.apply_typed_transaction(transaction).unwrap_err();

            assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{field}");
            assert_eq!(error.details, Some(json!({ "field": field })), "{field}");
            assert_eq!(atomic_audit(&engine), before, "{field}");
        }
    }

    #[test]
    fn compiled_transaction_epoch_is_checked_before_yrs_metadata_revalidation() {
        for changed in [true, false] {
            let mut engine = transaction_engine();
            let transaction = if changed {
                insert_transaction(&engine, 72)
            } else {
                TypedTransaction {
                    request_id: 72,
                    base_document_revision: engine.revision(),
                    origin: TransactionOrigin::LocalApi,
                    operations: vec![],
                    selection_intent: SelectionIntent::Preserve,
                    history_policy: HistoryPolicy::Skip,
                }
            };
            let compiled = engine.compile_typed_transaction(transaction).unwrap();
            engine.yrs_state_epoch += 1;
            let before = atomic_audit(&engine);

            let error = engine
                .apply_compiled_transaction(compiled, false)
                .unwrap_err();

            assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "changed={changed}");
            assert!(error.message.contains("stale"), "changed={changed}");
            assert_eq!(atomic_audit(&engine), before, "changed={changed}");
        }
    }

    #[test]
    fn compiled_transaction_state_revision_is_checked_before_result_or_no_op_work() {
        for changed in [true, false] {
            let mut engine = transaction_engine();
            let transaction = if changed {
                insert_transaction(&engine, 72_001)
            } else {
                TypedTransaction {
                    request_id: 72_001,
                    base_document_revision: engine.revision(),
                    origin: TransactionOrigin::LocalApi,
                    operations: vec![],
                    selection_intent: SelectionIntent::Preserve,
                    history_policy: HistoryPolicy::Skip,
                }
            };
            let compiled = engine.compile_typed_transaction(transaction).unwrap();
            let seed = engine
                .derived_state
                .as_ref()
                .unwrap()
                .mutation_lookup_seed
                .clone();
            engine
                .apply_typed_transaction(TypedTransaction {
                    request_id: 72_002,
                    base_document_revision: engine.revision(),
                    origin: TransactionOrigin::LocalApi,
                    operations: vec![],
                    selection_intent: SelectionIntent::Set(SelectionInput::All),
                    history_policy: HistoryPolicy::Skip,
                })
                .unwrap();
            assert!(Arc::ptr_eq(
                &seed,
                &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
            ));
            let before = atomic_audit(&engine);

            let error = engine
                .apply_compiled_transaction(compiled, true)
                .unwrap_err();

            assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "changed={changed}");
            assert!(error.message.contains("stale"), "changed={changed}");
            assert_eq!(atomic_audit(&engine), before, "changed={changed}");
        }
    }

    #[test]
    fn projected_encoded_ceiling_accepts_exact_and_rejects_one_under_without_new_clock() {
        let mut exact = transaction_engine();
        let exact_transaction = insert_transaction(&exact, 73);
        let exact_compiled = exact
            .compile_typed_transaction(exact_transaction.clone())
            .unwrap();
        let exact_limit = exact
            .encoded_state()
            .unwrap()
            .len()
            .checked_add(exact_compiled.encoded_growth_bound)
            .unwrap();
        exact.resource_limits.max_encoded_state_bytes = exact_limit;

        let commit = exact.apply_typed_transaction(exact_transaction).unwrap();

        assert!(commit.changed);
        assert!(exact.encoded_state().unwrap().len() <= exact_limit);

        let mut one_under = transaction_engine();
        let rejected_transaction = insert_transaction(&one_under, 74);
        let rejected_compiled = one_under
            .compile_typed_transaction(rejected_transaction.clone())
            .unwrap();
        let rejected_limit = one_under
            .encoded_state()
            .unwrap()
            .len()
            .checked_add(rejected_compiled.encoded_growth_bound)
            .unwrap()
            - 1;
        one_under.resource_limits.max_encoded_state_bytes = rejected_limit;
        let before = atomic_audit(&one_under);

        let error = one_under
            .apply_typed_transaction(rejected_transaction)
            .unwrap_err();

        assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
        assert_eq!(
            error.details,
            Some(json!({ "field": "maxEncodedStateBytes" }))
        );
        assert_eq!(error.limit, Some(rejected_limit as u64));
        assert_eq!(error.actual, Some((rejected_limit + 1) as u64));
        assert_eq!(atomic_audit(&one_under), before);
    }

    #[test]
    fn canonical_cache_output_accepts_exact_rejects_one_under_and_reuses_empty_noop_cache() {
        let expected = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{ "type": "text", "text": "x" }]
            }]
        });
        let exact_bytes = serde_json::to_vec(&expected).unwrap().len();

        let mut exact = transaction_engine();
        exact.editing_limits.max_derived_output_bytes = exact_bytes;
        let transaction = insert_transaction(&exact, 77);
        exact.apply_typed_transaction(transaction).unwrap();
        assert_eq!(exact.document_json(), Some(expected));

        let mut one_under = transaction_engine();
        one_under.editing_limits.max_derived_output_bytes = exact_bytes - 1;
        let transaction = insert_transaction(&one_under, 78);
        let before = atomic_audit(&one_under);
        let error = one_under.apply_typed_transaction(transaction).unwrap_err();
        assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
        assert_eq!(error.limit, Some((exact_bytes - 1) as u64));
        assert_eq!(error.actual, Some(exact_bytes as u64));
        assert_eq!(atomic_audit(&one_under), before);

        let mut empty_noop = transaction_engine();
        empty_noop.editing_limits.max_derived_output_bytes = 1;
        let transaction = TypedTransaction {
            request_id: 79,
            base_document_revision: empty_noop.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent: SelectionIntent::Preserve,
            history_policy: HistoryPolicy::Skip,
        };
        let before = atomic_audit(&empty_noop);
        let commit = empty_noop.apply_typed_transaction(transaction).unwrap();
        assert!(!commit.changed);
        assert_eq!(atomic_audit(&empty_noop), before);
    }

    #[test]
    fn local_empty_initialization_enforces_the_exact_canonical_output_ceiling() {
        let schema = tiptap_schema();
        let document = schema.default_document().unwrap();
        let value = crate::serialize::to_prosemirror_json(&document, &schema);
        let exact = serde_json::to_vec(&value).unwrap().len();
        let config = |limit| YrsEngineConfig {
            schema: tiptap_schema(),
            fragment_name: "prosemirror".into(),
            initialization_mode: crate::yrs_engine::InitializationMode::LocalEmpty,
            resource_limits: ResourceLimits::default(),
            editing_limits: crate::yrs_engine::EditingLimits {
                max_derived_output_bytes: limit,
                ..crate::yrs_engine::EditingLimits::default()
            },
            max_length: None,
            scope: None,
        };

        assert_eq!(
            YrsDocumentEngine::new(config(exact))
                .unwrap()
                .document_json(),
            Some(value)
        );
        let error = YrsDocumentEngine::new(config(exact - 1)).err().unwrap();
        assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
        assert_eq!(error.limit, Some(exact - 1));
        assert_eq!(error.actual, Some(exact));
        assert_eq!(
            error.details,
            Some(json!({ "field": "maxDerivedOutputBytes" }))
        );
    }

    #[test]
    fn json_and_html_import_enforce_output_before_any_live_state_change() {
        use crate::yrs_engine::canonical::{
            reset_canonical_artifact_counts_for_test, take_canonical_artifact_counts_for_test,
        };

        let expected = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{"type": "text", "text": "x"}]
            }]
        });
        let exact = serde_json::to_vec(&expected).unwrap().len();
        for (is_html, input) in [
            (false, serde_json::to_string(&expected).unwrap()),
            (true, "<p>x</p>".to_string()),
        ] {
            let mut accepted = transaction_engine();
            accepted.editing_limits.max_derived_output_bytes = exact;
            reset_canonical_artifact_counts_for_test();
            let commit = if is_html {
                accepted.import_html(
                    &input,
                    &FromHtmlOptions::default(),
                    TransactionOrigin::DocumentImport,
                )
            } else {
                accepted.import_json(&input, TransactionOrigin::DocumentImport)
            }
            .unwrap();
            assert!(commit.changed);
            assert_eq!(accepted.document_json(), Some(expected.clone()));
            assert_eq!(
                take_canonical_artifact_counts_for_test(),
                (1, usize::from(is_html))
            );

            let mut rejected = transaction_engine();
            rejected.editing_limits.max_derived_output_bytes = exact - 1;
            rejected.revision = u64::MAX;
            rejected.state_revision = u64::MAX;
            rejected.yrs_state_epoch = u64::MAX;
            rejected.derived_state.as_mut().unwrap().document_revision = u64::MAX;
            rejected.derived_state.as_mut().unwrap().state_revision = u64::MAX;
            let before = atomic_audit(&rejected);
            let artifact_before = rejected
                .derived_state
                .as_ref()
                .unwrap()
                .canonical_artifact
                .clone();
            reset_canonical_artifact_counts_for_test();
            let error = if is_html {
                rejected.import_html(
                    &input,
                    &FromHtmlOptions::default(),
                    TransactionOrigin::DocumentImport,
                )
            } else {
                rejected.import_json(&input, TransactionOrigin::DocumentImport)
            }
            .unwrap_err();
            assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED", "is_html={is_html}");
            assert_eq!(error.limit, Some(exact - 1));
            assert_eq!(error.actual, Some(exact));
            assert_eq!(
                error.details,
                Some(json!({ "field": "maxDerivedOutputBytes" }))
            );
            assert_eq!(atomic_audit(&rejected), before);
            assert!(artifact_before
                .ptr_eq(&rejected.derived_state.as_ref().unwrap().canonical_artifact));
            assert_eq!(take_canonical_artifact_counts_for_test(), (1, 1));
        }
    }

    #[test]
    fn changed_snapshot_restore_enforces_output_before_revisions_history_or_swap() {
        use crate::yrs_engine::canonical::{
            reset_canonical_artifact_counts_for_test, take_canonical_artifact_counts_for_test,
        };

        let mut source = transaction_engine();
        source
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"snapshot"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let snapshot = source.export_snapshot().unwrap();
        let exact = serde_json::to_vec(&source.document_json().unwrap())
            .unwrap()
            .len();

        let mut accepted = transaction_engine();
        accepted.editing_limits.max_derived_output_bytes = exact;
        reset_canonical_artifact_counts_for_test();
        assert!(accepted.restore_snapshot(&snapshot).unwrap().changed);
        assert_eq!(take_canonical_artifact_counts_for_test(), (1, 2));
        accepted.editing_limits.max_derived_output_bytes = 1;
        reset_canonical_artifact_counts_for_test();
        assert!(!accepted.restore_snapshot(&snapshot).unwrap().changed);
        assert_eq!(take_canonical_artifact_counts_for_test(), (0, 0));

        let mut rejected = transaction_engine();
        rejected.editing_limits.max_derived_output_bytes = exact - 1;
        rejected.revision = u64::MAX;
        rejected.state_revision = u64::MAX;
        rejected.yrs_state_epoch = u64::MAX;
        rejected.derived_state.as_mut().unwrap().document_revision = u64::MAX;
        rejected.derived_state.as_mut().unwrap().state_revision = u64::MAX;
        let before = atomic_audit(&rejected);
        let artifact_before = rejected
            .derived_state
            .as_ref()
            .unwrap()
            .canonical_artifact
            .clone();
        reset_canonical_artifact_counts_for_test();
        let error = rejected.restore_snapshot(&snapshot).unwrap_err();
        assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
        assert_eq!(error.limit, Some(exact - 1));
        assert_eq!(error.actual, Some(exact));
        assert_eq!(
            error.details,
            Some(json!({ "field": "maxDerivedOutputBytes" }))
        );
        assert_eq!(atomic_audit(&rejected), before);
        assert!(
            artifact_before.ptr_eq(&rejected.derived_state.as_ref().unwrap().canonical_artifact)
        );
        assert_eq!(take_canonical_artifact_counts_for_test(), (1, 1));
    }

    #[test]
    fn typed_commit_installs_local_client_origin_and_candidate_revisions() {
        let mut source = transaction_engine();
        let imported = source
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"seed"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        assert!(imported.changed);
        assert_eq!(
            (
                source.revision,
                source.state_revision,
                source.yrs_state_epoch
            ),
            (1, 1, 1)
        );
        let snapshot = source.export_snapshot().unwrap();
        let mut target = transaction_engine();
        target.restore_snapshot(&snapshot).unwrap();
        let local_client = target.client_id();
        assert!(!target.durable_client_ids.contains(&local_client));
        assert_eq!(
            (
                target.revision,
                target.state_revision,
                target.yrs_state_epoch
            ),
            (1, 1, 1)
        );

        let transaction = insert_transaction(&target, 75);
        let commit = target.apply_typed_transaction(transaction).unwrap();

        assert!(commit.changed);
        assert!(target.durable_client_ids.contains(&local_client));
        assert_eq!(
            target.last_committed_origin,
            Some(TransactionOrigin::LocalApi)
        );
        assert_eq!(
            (
                target.revision,
                target.state_revision,
                target.yrs_state_epoch
            ),
            (2, 2, 2)
        );

        let unchanged = target.document_json().unwrap();
        let commit = target
            .import_json(
                &serde_json::to_string(&unchanged).unwrap(),
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        assert!(!commit.changed);
        assert_eq!(
            (
                target.revision,
                target.state_revision,
                target.yrs_state_epoch
            ),
            (2, 2, 2)
        );
    }

    #[test]
    fn restored_deletion_only_commit_does_not_claim_an_unauthored_local_client() {
        let mut source = transaction_engine();
        source
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"seed"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let snapshot = source.export_snapshot().unwrap();
        let mut target = transaction_engine();
        target.restore_snapshot(&snapshot).unwrap();
        let local_client = target.client_id();
        assert!(!target.durable_client_ids.contains(&local_client));
        let from = RevisionedPosition {
            offset: 0,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::After,
        };
        let to = RevisionedPosition { offset: 1, ..from };
        let transaction = TypedTransaction {
            request_id: 80,
            base_document_revision: target.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![TypedOperation::DeleteRange {
                range: RevisionedRange { from, to },
            }],
            selection_intent: SelectionIntent::Preserve,
            history_policy: HistoryPolicy::Skip,
        };

        let compiled = target
            .compile_typed_transaction(transaction.clone())
            .unwrap();
        assert_eq!(compiled.authored_clock_units, 0);
        target.apply_typed_transaction(transaction).unwrap();

        assert_prepared_candidate_state_vector_exact(&target);
        assert!(!target.durable_client_ids.contains(&local_client));
        let durable_clients = Update::decode_v1(&target.encoded_state().unwrap())
            .unwrap()
            .state_vector();
        assert!(durable_clients.get(&ClientID::new(local_client)) == 0);
    }

    #[test]
    fn restored_format_only_commit_records_its_authored_local_clock() {
        let mut source = transaction_engine();
        source
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"seed"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let snapshot = source.export_snapshot().unwrap();
        let mut target = transaction_engine();
        target.restore_snapshot(&snapshot).unwrap();
        let local_client = target.client_id();
        let from = RevisionedPosition {
            offset: 0,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::After,
        };
        let to = RevisionedPosition { offset: 1, ..from };
        let transaction = TypedTransaction {
            request_id: 81,
            base_document_revision: target.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![TypedOperation::AddMark {
                range: RevisionedRange { from, to },
                mark: Mark::new("bold".into(), HashMap::new()),
            }],
            selection_intent: SelectionIntent::Preserve,
            history_policy: HistoryPolicy::Skip,
        };

        let compiled = target
            .compile_typed_transaction(transaction.clone())
            .unwrap();
        assert!(compiled.authored_clock_units > 0);
        target.apply_typed_transaction(transaction).unwrap();

        assert_prepared_candidate_state_vector_exact(&target);
        assert!(target.durable_client_ids.contains(&local_client));
        let durable_clients = Update::decode_v1(&target.encoded_state().unwrap())
            .unwrap()
            .state_vector();
        assert!(durable_clients.get(&ClientID::new(local_client)) > 0);
    }

    fn select_text(engine: &mut YrsDocumentEngine, request_id: u64, anchor: u32, head: u32) {
        let point = |offset| RevisionedPosition {
            offset,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::Before,
        };
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: vec![],
                selection_intent: SelectionIntent::Set(SelectionInput::Text {
                    anchor: point(anchor),
                    head: point(head),
                }),
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap();
    }

    fn unaffected_text_sticky(
        engine: &YrsDocumentEngine,
        text_child: u32,
        utf16_index: u32,
    ) -> (crate::yrs_engine::RelativePoint, BranchPtr, u32) {
        let txn = engine.doc.transact();
        let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
        let XmlOut::Element(paragraph) = fragment.get(&txn, 0).unwrap() else {
            panic!("expected paragraph")
        };
        let XmlOut::Text(text) = paragraph.get(&txn, text_child).unwrap() else {
            panic!("expected text child")
        };
        let branch = BranchPtr::from(<XmlTextRef as AsRef<Branch>>::as_ref(&text));
        let sticky = StickyIndex::at(&txn, branch, utf16_index, Assoc::After).unwrap();
        let point = crate::yrs_engine::RelativePoint {
            sticky,
            affinity: Affinity::After,
        };
        let Some(offset) = point.sticky.get_offset(&txn) else {
            panic!("sticky must resolve")
        };
        let doc_pos = crate::yrs_engine::position::relative_point_to_doc_pos(
            &txn,
            &fragment,
            &point,
            &engine.schema,
        )
        .unwrap();
        let scalar = engine
            .position_map()
            .unwrap()
            .doc_to_scalar(doc_pos, engine.document().unwrap());
        (point, offset.branch, scalar)
    }

    fn assert_unaffected_sticky(
        engine: &YrsDocumentEngine,
        point: &crate::yrs_engine::RelativePoint,
        branch: BranchPtr,
        expected_scalar: u32,
    ) {
        let txn = engine.doc.transact();
        let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
        let offset = point.sticky.get_offset(&txn).unwrap();
        assert_eq!(
            offset.branch, branch,
            "unaffected Yrs branch identity changed"
        );
        let doc_pos = crate::yrs_engine::position::relative_point_to_doc_pos(
            &txn,
            &fragment,
            point,
            &engine.schema,
        )
        .unwrap();
        assert_eq!(
            engine
                .position_map()
                .unwrap()
                .doc_to_scalar(doc_pos, engine.document().unwrap()),
            expected_scalar,
            "unaffected sticky point moved to the wrong rendered position"
        );
    }

    #[test]
    fn granular_command_lowering_preserves_classification_locality_and_unaffected_sticky_identity()
    {
        let mut format = transaction_engine();
        format
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"a","marks":[{"type":"link","attrs":{"href":"old"}}]},{"type":"text","text":"bc tail"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let (format_sticky, format_branch, format_scalar) = unaffected_text_sticky(&format, 1, 5);
        select_text(&mut format, 100, 2, 0);
        let CommandPlan::Transaction(format_transaction) = format
            .plan_command(
                101,
                TypedCommand::SetMark {
                    mark_type: "link".into(),
                    attrs: HashMap::from([("href".into(), json!("new"))]),
                },
            )
            .unwrap()
        else {
            panic!("range format must plan")
        };
        assert!(matches!(
            format_transaction.operations.as_slice(),
            [
                TypedOperation::RemoveMark { .. },
                TypedOperation::AddMark { .. }
            ]
        ));
        let compiled = format
            .compile_typed_transaction(format_transaction.clone())
            .unwrap();
        assert_eq!(
            compiled.history_class,
            crate::yrs_engine::compiler::HistoryClass::Format
        );
        assert_eq!(
            compiled.position_update_mode,
            crate::position::update::UpdateMode::MarksOnly
        );
        assert_eq!(compiled.affected_top_level_blocks, vec![0]);
        let format_result = format
            .apply_typed_transaction_with_result(format_transaction)
            .unwrap();
        let crate::yrs_engine::RenderUpdate::Patch(format_patch) = format_result.render_update
        else {
            panic!("range format must produce a local render patch")
        };
        assert_eq!(
            (
                format_patch.start_index,
                format_patch.delete_count,
                format_patch.blocks.len(),
            ),
            (0, 1, 1)
        );
        assert_unaffected_sticky(&format, &format_sticky, format_branch, format_scalar);
        assert!(format.can_undo());

        let mut replace = transaction_engine();
        replace
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"left target right"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let (replace_sticky, replace_branch, replace_scalar) =
            unaffected_text_sticky(&replace, 0, 13);
        select_text(&mut replace, 102, 11, 5);
        let CommandPlan::Transaction(replace_transaction) = replace
            .plan_command(
                103,
                TypedCommand::ReplaceSelectionText { text: "new".into() },
            )
            .unwrap()
        else {
            panic!("range replacement must plan")
        };
        assert!(matches!(
            replace_transaction.operations.as_slice(),
            [
                TypedOperation::DeleteRange { .. },
                TypedOperation::InsertText { .. }
            ]
        ));
        let compiled = replace
            .compile_typed_transaction(replace_transaction.clone())
            .unwrap();
        assert_eq!(
            compiled.history_class,
            crate::yrs_engine::compiler::HistoryClass::Structural
        );
        assert_eq!(
            compiled.position_update_mode,
            crate::position::update::UpdateMode::InlineTextOnly
        );
        assert_eq!(compiled.affected_top_level_blocks, vec![0]);
        let replace_result = replace
            .apply_typed_transaction_with_result(replace_transaction)
            .unwrap();
        let crate::yrs_engine::RenderUpdate::Patch(replace_patch) = replace_result.render_update
        else {
            panic!("range replacement must produce a local render patch")
        };
        assert_eq!(
            (
                replace_patch.start_index,
                replace_patch.delete_count,
                replace_patch.blocks.len(),
            ),
            (0, 1, 1)
        );
        assert_unaffected_sticky(
            &replace,
            &replace_sticky,
            replace_branch,
            replace_scalar - 3,
        );
        assert!(replace.can_undo());
    }

    #[test]
    fn typed_edits_advance_cached_render_blocks_while_selection_only_retains_arc() {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"one"}]},{"type":"paragraph","content":[{"type":"text","text":"two"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let initial = Arc::clone(&engine.derived_state.as_ref().unwrap().render_blocks);
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 104,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: vec![],
                selection_intent: SelectionIntent::Set(SelectionInput::All),
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap();
        assert!(Arc::ptr_eq(
            &initial,
            &engine.derived_state.as_ref().unwrap().render_blocks
        ));

        let old_blocks = initial.materialize();
        crate::render::incremental::reset_cached_render_counts_for_test();
        let result = engine
            .apply_typed_transaction_with_result(TypedTransaction {
                request_id: 105,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: vec![TypedOperation::InsertText {
                    at: RevisionedPosition {
                        offset: 1,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    text: "x".into(),
                    marks: vec![],
                }],
                selection_intent: SelectionIntent::Preserve,
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap();
        assert_eq!(
            crate::render::incremental::take_cached_render_counts_for_test(),
            (0, 1, 1, 0, 0)
        );
        let next = engine.derived_state.as_ref().unwrap();
        assert!(!Arc::ptr_eq(&initial, &next.render_blocks));

        let reconstructed = match result.render_update {
            crate::yrs_engine::RenderUpdate::None => old_blocks,
            crate::yrs_engine::RenderUpdate::Full(blocks) => blocks,
            crate::yrs_engine::RenderUpdate::Patch(patch) => {
                let mut blocks = old_blocks;
                blocks.splice(
                    patch.start_index..patch.start_index + patch.delete_count,
                    patch.blocks,
                );
                blocks
            }
        };
        assert_eq!(reconstructed, next.render_blocks.materialize());
        assert_eq!(
            next.render_blocks.materialize(),
            crate::render::incremental::render_blocks(&next.document, &engine.schema)
        );
    }

    #[test]
    fn history_results_compare_sealed_render_caches_without_full_old_new_render() {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"one"}]},{"type":"paragraph","content":[{"type":"text","text":"two"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let before_edit_cache = Arc::clone(
            &engine
                .derived_state
                .as_ref()
                .expect("import initializes derived state")
                .render_blocks,
        );
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 106,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: vec![TypedOperation::InsertText {
                    at: RevisionedPosition {
                        offset: 1,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    text: "x".into(),
                    marks: vec![],
                }],
                selection_intent: SelectionIntent::Preserve,
                history_policy: HistoryPolicy::Auto,
            })
            .unwrap();
        let after_edit_cache = Arc::clone(
            &engine
                .derived_state
                .as_ref()
                .expect("edit initializes derived state")
                .render_blocks,
        );

        let before_undo = engine
            .derived_state
            .as_ref()
            .unwrap()
            .render_blocks
            .materialize();
        crate::render::incremental::reset_cached_render_counts_for_test();
        let undo = engine.undo_with_result(107).unwrap().unwrap();
        assert_eq!(
            crate::render::incremental::take_cached_render_counts_for_test(),
            (0, 0, 0, 0, 0)
        );
        let after_undo = engine.derived_state.as_ref().unwrap();
        assert!(Arc::ptr_eq(&before_edit_cache, &after_undo.render_blocks));
        let reconstructed = apply_render_update_for_test(before_undo, undo.render_update);
        assert_eq!(reconstructed, after_undo.render_blocks.materialize());

        let before_redo = after_undo.render_blocks.materialize();
        crate::render::incremental::reset_cached_render_counts_for_test();
        let redo = engine.redo_with_result(108).unwrap().unwrap();
        assert_eq!(
            crate::render::incremental::take_cached_render_counts_for_test(),
            (0, 0, 0, 0, 0)
        );
        let after_redo = engine.derived_state.as_ref().unwrap();
        assert!(Arc::ptr_eq(&after_edit_cache, &after_redo.render_blocks));
        assert_eq!(
            apply_render_update_for_test(before_redo, redo.render_update),
            after_redo.render_blocks.materialize()
        );
    }

    #[test]
    fn history_snapshot_seed_publication_errors_propagate_real_request_atomically() {
        use crate::yrs_engine::mutation::{
            set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
        };
        use crate::yrs_engine::observability::{
            reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
        };

        for (request_id, failpoint, expected_stage) in [
            (
                108_056,
                LookupSeedHydrationFailpoint::BindingPublication,
                "historyStoreSnapshotPublication",
            ),
            (
                108_057,
                LookupSeedHydrationFailpoint::SeedPublication,
                "historyUnavailableSeedPublication",
            ),
        ] {
            let mut engine = transaction_engine();
            engine
                .import_json(
                    r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                    TransactionOrigin::DocumentImport,
                )
                .unwrap();
            engine
                .apply_typed_transaction(TypedTransaction {
                    request_id: request_id - 1,
                    base_document_revision: engine.revision(),
                    origin: TransactionOrigin::LocalInput,
                    operations: vec![TypedOperation::InsertText {
                        at: RevisionedPosition {
                            offset: 2,
                            kind: EditorOffsetKind::Scalar,
                            affinity: Affinity::After,
                        },
                        text: "x".into(),
                        marks: vec![],
                    }],
                    selection_intent: SelectionIntent::UseOperationResult,
                    history_policy: HistoryPolicy::Boundary,
                })
                .unwrap();
            assert!(engine.can_undo());
            assert!(engine
                .derived_state
                .as_ref()
                .unwrap()
                .mutation_lookup_seed
                .is_ready_for_test());
            let before = atomic_audit(&engine);
            let installed =
                Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);

            reset_prepared_admission_counts_for_test();
            set_lookup_seed_hydration_failpoint_for_test(Some(failpoint));
            let result = engine.undo_with_result(request_id);
            set_lookup_seed_hydration_failpoint_for_test(None);

            let error = result.expect_err("history snapshot publication failure must propagate");
            assert_eq!(error.code, "OPERATION_RESOURCE_EXHAUSTED");
            assert_eq!(error.request_id, request_id);
            assert_eq!(
                error.message.as_ref(),
                format!("mutation lookup seed allocation failed during {expected_stage}")
            );
            assert_eq!(
                error.details,
                Some(json!({ "field": "mutationLookupSeed" }))
            );
            let counts = take_prepared_admission_counts_for_test();
            assert_eq!(counts.staged_seed_preparations, 0);
            assert_eq!(counts.installed_base_seed_publications, 0);
            assert_eq!(atomic_audit(&engine), before);
            assert!(Arc::ptr_eq(
                &installed,
                &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
            ));
        }
    }

    #[test]
    fn history_snapshot_equality_uses_document_snapshot_arc_identity() {
        let engine = transaction_engine();
        let state = engine.derived_state.as_ref().unwrap();
        let retained = crate::yrs_engine::derived_state::history_document_snapshot_retained_bytes(
            crate::yrs_engine::derived_state::HistoryDocumentSnapshotRetainedInput {
                document: &state.document,
                canonical_artifact: &state.canonical_artifact,
                position_map: &state.position_map,
                rendered_text: &state.rendered_text,
                render_blocks: &state.render_blocks,
                schema_fingerprint: &engine.schema_fingerprint,
                fragment_name: &engine.fragment_name,
                scope: engine.scope.as_ref(),
            },
        )
        .unwrap();
        let document_snapshot = state.capture_history_document_snapshot(
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            &engine.fragment_name,
            engine.scope.as_ref(),
            retained,
        );
        let snapshot = crate::yrs_engine::history::HistorySnapshot {
            relative_selection: state.relative_selection.clone(),
            resolved_selection: state.resolved_selection.clone(),
            stored_marks: state.stored_marks.clone(),
            text_length: state.canonical_artifact.text_scalar_len(),
            canonical_fingerprint: state.canonical_artifact.sha256(),
            derived_output_bytes: state.canonical_artifact.serialized_len(),
            metadata_bytes: retained.get(),
            document_snapshot: Some(document_snapshot),
        };
        let shared = snapshot.clone();
        assert_eq!(snapshot, shared);

        let mut equivalent_but_distinct = snapshot.clone();
        let document_snapshot = snapshot
            .document_snapshot
            .as_ref()
            .expect("default article history retains its document snapshot");
        equivalent_but_distinct.document_snapshot = Some(Arc::new((**document_snapshot).clone()));
        assert_ne!(snapshot, equivalent_but_distinct);
    }

    #[test]
    fn history_restoration_resolves_only_the_popped_selection_without_a_default_roundtrip() {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let before_json = engine.document_json().unwrap();
        let before_selection = engine.resolved_selection().cloned().unwrap();
        let before_marks = engine.stored_marks().map(<[_]>::to_vec);
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 108_001,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: RevisionedPosition {
                        offset: 2,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    text: "x".into(),
                    marks: vec![],
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Boundary,
            })
            .unwrap();
        let after_json = engine.document_json().unwrap();
        let after_selection = engine.resolved_selection().cloned().unwrap();
        let after_marks = engine.stored_marks().map(<[_]>::to_vec);

        for (request_id, undoing) in [(108_002, true), (108_003, false)] {
            crate::yrs_engine::derived_state::reset_relative_selection_traversal_counts_for_test();
            crate::yrs_engine::observability::reset_full_pass_counts_for_test();

            if undoing {
                engine.undo_with_result(request_id).unwrap().unwrap();
            } else {
                engine.redo_with_result(request_id).unwrap().unwrap();
            }

            let (expected_json, expected_selection, expected_marks) = if undoing {
                (&before_json, &before_selection, &before_marks)
            } else {
                (&after_json, &after_selection, &after_marks)
            };
            assert_eq!(engine.document_json().as_ref(), Some(expected_json));
            assert_eq!(engine.resolved_selection(), Some(expected_selection));
            assert_eq!(
                engine.stored_marks().map(<[_]>::to_vec).as_ref(),
                expected_marks.as_ref()
            );

            assert_eq!(
                crate::yrs_engine::derived_state::take_relative_selection_traversal_counts_for_test(
                ),
                (1, 1),
                "history restoration should materialize only the exact popped selection"
            );
            let full_passes = crate::yrs_engine::observability::take_full_pass_counts_for_test();
            // The document-scoped history snapshot is admitted by exact
            // candidate JSON equality, so no canonical projection,
            // serialization, or hash pass is repeated during the pop.
            assert_eq!(full_passes.canonical_projections, 0);
            assert_eq!(full_passes.canonical_serializations, 0);
            assert_eq!(full_passes.canonical_hashes, 0);
        }
    }

    #[test]
    fn tight_history_metadata_budget_falls_back_to_full_candidate_derivation() {
        let mut engine = transaction_engine_with_editing_limits(crate::yrs_engine::EditingLimits {
            max_derived_output_bytes: 2 * (512 + "prosemirror".len() + 2),
            ..crate::yrs_engine::EditingLimits::default()
        });
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 108_004,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: RevisionedPosition {
                        offset: 1,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    text: "x".into(),
                    marks: vec![],
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Boundary,
            })
            .unwrap();

        crate::yrs_engine::observability::reset_full_pass_counts_for_test();
        engine.undo_with_result(108_005).unwrap().unwrap();

        assert_eq!(
            engine.document_json().unwrap(),
            serde_json::from_str::<serde_json::Value>(
                r#"{"type":"doc","content":[{"type":"paragraph"}]}"#,
            )
            .unwrap()
        );
        let full_passes = crate::yrs_engine::observability::take_full_pass_counts_for_test();
        assert!(full_passes.canonical_projections > 0);
        assert!(full_passes.canonical_serializations > 0);
        assert!(full_passes.canonical_hashes > 0);
    }

    #[test]
    fn deep_wide_history_snapshot_budget_accounts_for_spilled_position_paths() {
        fn deep_wide_document() -> serde_json::Value {
            let mut content = (0..24)
                .map(|index| {
                    json!({
                        "type": "paragraph",
                        "content": [{"type": "text", "text": format!("row {index}")}]
                    })
                })
                .collect::<Vec<_>>();
            for _ in 0..10 {
                content = vec![json!({"type": "blockquote", "content": content})];
            }
            json!({"type": "doc", "content": content})
        }

        fn insert(engine: &YrsDocumentEngine, request_id: u64) -> TypedTransaction {
            TypedTransaction {
                request_id,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: RevisionedPosition {
                        offset: 1,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    text: "x".into(),
                    marks: vec![],
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Boundary,
            }
        }

        let document = deep_wide_document();
        let mut probe = transaction_engine();
        probe
            .import_json(&document.to_string(), TransactionOrigin::DocumentImport)
            .unwrap();
        let compiled = probe
            .compile_typed_transaction(insert(&probe, 108_006))
            .unwrap();
        let after = compiled.preview_derivations.as_ref().unwrap();
        let before = probe.derived_state.as_ref().unwrap();
        let before_retained =
            crate::yrs_engine::derived_state::history_document_snapshot_retained_bytes(
                crate::yrs_engine::derived_state::HistoryDocumentSnapshotRetainedInput {
                    document: &before.document,
                    canonical_artifact: &before.canonical_artifact,
                    position_map: &before.position_map,
                    rendered_text: &before.rendered_text,
                    render_blocks: &before.render_blocks,
                    schema_fingerprint: &probe.schema_fingerprint,
                    fragment_name: &probe.fragment_name,
                    scope: probe.scope.as_ref(),
                },
            )
            .unwrap();
        let after_retained =
            crate::yrs_engine::derived_state::history_document_snapshot_retained_bytes(
                crate::yrs_engine::derived_state::HistoryDocumentSnapshotRetainedInput {
                    document: &compiled.preview,
                    canonical_artifact: compiled.canonical_artifact.as_ref().unwrap(),
                    position_map: &after.position_map,
                    rendered_text: &after.rendered_text,
                    render_blocks: &crate::render::incremental::CachedRenderBlocks::build(
                        &compiled.preview,
                        &probe.schema,
                        &probe.resource_limits,
                    )
                    .unwrap(),
                    schema_fingerprint: &probe.schema_fingerprint,
                    fragment_name: &probe.fragment_name,
                    scope: probe.scope.as_ref(),
                },
            )
            .unwrap();
        let exact_budget =
            super::history_metadata_bytes(before.stored_marks.as_deref(), &probe.fragment_name)
                .checked_add(super::history_metadata_bytes(None, &probe.fragment_name))
                .and_then(|bytes| bytes.checked_add(before_retained.get()))
                .and_then(|bytes| bytes.checked_add(after_retained.get()))
                .unwrap();

        let run = |limit, request_id| {
            let mut engine = transaction_engine();
            engine
                .import_json(&document.to_string(), TransactionOrigin::DocumentImport)
                .unwrap();
            engine.editing_limits.max_derived_output_bytes = limit;
            engine
                .apply_typed_transaction(insert(&engine, request_id))
                .unwrap();
            assert!(
                engine.can_undo(),
                "base history capture must remain admitted"
            );
            crate::yrs_engine::observability::reset_full_pass_counts_for_test();
            engine.undo_with_result(request_id + 1).unwrap().unwrap();
            crate::yrs_engine::observability::take_full_pass_counts_for_test()
        };

        let exact_passes = run(exact_budget, 108_007);
        assert_eq!(
            exact_passes.canonical_projections, 0,
            "the exact retained bound should admit the optional snapshots"
        );

        let full_passes = run(exact_budget - 1, 108_009);
        assert!(
            full_passes.canonical_projections > 0,
            "one under the retained bound must omit only the optional snapshots"
        );
    }

    #[test]
    fn history_snapshot_charge_tracks_spare_node_string_capacity() {
        const SPARE_CAPACITY: usize = 1024 * 1024;

        fn fixture(limit: usize) -> YrsDocumentEngine {
            let mut engine =
                transaction_engine_with_editing_limits(crate::yrs_engine::EditingLimits {
                    max_derived_output_bytes: limit,
                    ..crate::yrs_engine::EditingLimits::default()
                });
            engine
                .import_json(
                    r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"ab"}]}]}"#,
                    TransactionOrigin::DocumentImport,
                )
                .unwrap();
            engine
        }

        fn transaction(engine: &YrsDocumentEngine, request_id: u64) -> TypedTransaction {
            let mut node_type = String::with_capacity(SPARE_CAPACITY);
            node_type.push_str("hardBreak");
            TypedTransaction {
                request_id,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertNode {
                    at: RevisionedPosition {
                        offset: 1,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    node: crate::model::Node::void(node_type, HashMap::new()),
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Boundary,
            }
        }

        fn snapshot_charge(engine: &YrsDocumentEngine) -> usize {
            let state = engine.derived_state.as_ref().unwrap();
            crate::yrs_engine::derived_state::history_document_snapshot_retained_bytes(
                crate::yrs_engine::derived_state::HistoryDocumentSnapshotRetainedInput {
                    document: &state.document,
                    canonical_artifact: &state.canonical_artifact,
                    position_map: &state.position_map,
                    rendered_text: &state.rendered_text,
                    render_blocks: &state.render_blocks,
                    schema_fingerprint: &engine.schema_fingerprint,
                    fragment_name: &engine.fragment_name,
                    scope: engine.scope.as_ref(),
                },
            )
            .unwrap()
            .get()
        }

        let before_probe =
            fixture(crate::yrs_engine::EditingLimits::default().max_derived_output_bytes);
        let before_charge = snapshot_charge(&before_probe);
        let before_metadata =
            super::history_metadata_bytes(before_probe.stored_marks(), &before_probe.fragment_name);
        let mut after_probe =
            fixture(crate::yrs_engine::EditingLimits::default().max_derived_output_bytes);
        after_probe
            .apply_typed_transaction(transaction(&after_probe, 108_020))
            .unwrap();
        let after_charge = snapshot_charge(&after_probe);
        assert!(after_charge >= SPARE_CAPACITY);
        let exact = before_metadata
            .checked_add(super::history_metadata_bytes(
                after_probe.stored_marks(),
                &after_probe.fragment_name,
            ))
            .and_then(|bytes| bytes.checked_add(before_charge))
            .and_then(|bytes| bytes.checked_add(after_charge))
            .unwrap();

        for (limit, expect_fast, request_id) in
            [(exact, true, 108_021), (exact - 1, false, 108_023)]
        {
            let mut engine = fixture(limit);
            engine
                .apply_typed_transaction(transaction(&engine, request_id))
                .unwrap();
            assert!(engine.can_undo());
            crate::yrs_engine::observability::reset_full_pass_counts_for_test();
            engine.undo_with_result(request_id + 1).unwrap().unwrap();
            let passes = crate::yrs_engine::observability::take_full_pass_counts_for_test();
            assert_eq!(passes.canonical_projections == 0, expect_fast);
        }
    }

    #[test]
    fn stored_mark_metadata_accounts_spare_hash_capacity_at_exact_boundary() {
        const SPARE_ENTRIES: usize = 32 * 1024;

        fn fixture(limit: usize) -> YrsDocumentEngine {
            let mut engine =
                transaction_engine_with_editing_limits(crate::yrs_engine::EditingLimits {
                    max_derived_output_bytes: limit,
                    ..crate::yrs_engine::EditingLimits::default()
                });
            engine
                .import_json(
                    r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"ab"}]}]}"#,
                    TransactionOrigin::DocumentImport,
                )
                .unwrap();
            select_text(&mut engine, 108_030, 1, 1);
            let mut attrs = HashMap::with_capacity(SPARE_ENTRIES);
            attrs.insert("href".into(), json!("x"));
            engine
                .apply_command(
                    108_031,
                    TypedCommand::SetMark {
                        mark_type: "link".into(),
                        attrs,
                    },
                )
                .unwrap()
                .unwrap();
            assert!(engine.stored_marks().is_some());
            engine
        }

        fn transaction(engine: &YrsDocumentEngine, request_id: u64) -> TypedTransaction {
            TypedTransaction {
                request_id,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: RevisionedPosition {
                        offset: 1,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    text: "x".into(),
                    marks: engine.stored_marks().unwrap().to_vec(),
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Boundary,
            }
        }

        let mut probe =
            fixture(crate::yrs_engine::EditingLimits::default().max_derived_output_bytes);
        let before_metadata =
            super::history_metadata_bytes(probe.stored_marks(), &probe.fragment_name);
        probe
            .apply_typed_transaction(transaction(&probe, 108_032))
            .unwrap();
        let exact = before_metadata
            .checked_add(super::history_metadata_bytes(
                probe.stored_marks(),
                &probe.fragment_name,
            ))
            .unwrap();

        let mut accepted = fixture(exact);
        accepted
            .apply_typed_transaction(transaction(&accepted, 108_033))
            .unwrap();
        assert!(accepted.can_undo());

        let mut rejected = fixture(exact - 1);
        let before = atomic_audit(&rejected);
        let error = rejected
            .apply_typed_transaction(transaction(&rejected, 108_034))
            .unwrap_err();
        assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
        assert_eq!(atomic_audit(&rejected), before);
    }

    #[test]
    fn compatible_auto_capture_admits_exact_after_only_metadata_increment() {
        fn fixture(limit: usize) -> YrsDocumentEngine {
            let mut engine =
                transaction_engine_with_editing_limits(crate::yrs_engine::EditingLimits {
                    max_derived_output_bytes: limit,
                    ..crate::yrs_engine::EditingLimits::default()
                });
            engine
                .import_json(
                    r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"ab"}]}]}"#,
                    TransactionOrigin::DocumentImport,
                )
                .unwrap();
            engine
        }

        fn insert(engine: &YrsDocumentEngine, request_id: u64, offset: u32) -> TypedTransaction {
            TypedTransaction {
                request_id,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: RevisionedPosition {
                        offset,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    text: "x".into(),
                    marks: vec![],
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            }
        }

        let default_limit = crate::yrs_engine::EditingLimits::default().max_derived_output_bytes;
        let mut probe = fixture(default_limit);
        probe
            .apply_typed_transaction(insert(&probe, 108_040, 1))
            .unwrap();
        let retained_before_second = probe.history.replay_metadata_bytes_for_test();
        let second_before_metadata = {
            let state = probe.derived_state.as_ref().unwrap();
            let retained =
                crate::yrs_engine::derived_state::history_document_snapshot_retained_bytes(
                    crate::yrs_engine::derived_state::HistoryDocumentSnapshotRetainedInput {
                        document: &state.document,
                        canonical_artifact: &state.canonical_artifact,
                        position_map: &state.position_map,
                        rendered_text: &state.rendered_text,
                        render_blocks: &state.render_blocks,
                        schema_fingerprint: &probe.schema_fingerprint,
                        fragment_name: &probe.fragment_name,
                        scope: probe.scope.as_ref(),
                    },
                )
                .unwrap()
                .get();
            super::history_metadata_bytes(probe.stored_marks(), &probe.fragment_name)
                .checked_add(retained)
                .unwrap()
        };
        probe
            .apply_typed_transaction(insert(&probe, 108_041, 2))
            .unwrap();
        let second_after_metadata = probe
            .history
            .replay_metadata_bytes_for_test()
            .checked_sub(retained_before_second)
            .unwrap();
        let exact = retained_before_second
            .checked_add(second_after_metadata)
            .unwrap();
        assert!(
            exact
                < retained_before_second
                    .checked_add(second_before_metadata)
                    .and_then(|bytes| bytes.checked_add(second_after_metadata))
                    .unwrap()
        );

        let mut engine = fixture(exact);
        engine
            .apply_typed_transaction(insert(&engine, 108_042, 1))
            .unwrap();
        engine
            .apply_typed_transaction(insert(&engine, 108_043, 2))
            .unwrap();
        assert_eq!(engine.document().unwrap().root().text_content(), "axxb");
        crate::yrs_engine::observability::reset_full_pass_counts_for_test();
        engine.undo_with_result(108_044).unwrap().unwrap();
        assert_eq!(engine.document().unwrap().root().text_content(), "ab");
        assert!(!engine.can_undo(), "compatible edits must remain one group");
        assert_eq!(
            crate::yrs_engine::observability::take_full_pass_counts_for_test()
                .canonical_projections,
            0,
            "the exact boundary keeps optional document snapshots enabled"
        );
    }

    #[test]
    fn history_snapshot_and_forced_fallback_match_affinity_and_stored_marks() {
        use crate::yrs_engine::derived_state::force_history_document_snapshot_fallback_for_test;

        fn fixture() -> YrsDocumentEngine {
            let mut engine = transaction_engine();
            engine
                .import_json(
                    &json!({
                        "type": "doc",
                        "content": [
                            {"type": "paragraph", "content": [{"type": "text", "text": "a😀b"}]},
                            {"type": "horizontalRule"},
                            {"type": "paragraph", "content": [{"type": "text", "text": "tail"}]}
                        ]
                    })
                    .to_string(),
                    TransactionOrigin::DocumentImport,
                )
                .unwrap();
            let boundary = |affinity| RevisionedPosition {
                offset: 2,
                kind: EditorOffsetKind::Scalar,
                affinity,
            };
            engine
                .apply_typed_transaction(TypedTransaction {
                    request_id: 108_052,
                    base_document_revision: engine.revision(),
                    origin: TransactionOrigin::LocalApi,
                    operations: vec![],
                    selection_intent: SelectionIntent::Set(SelectionInput::Text {
                        anchor: boundary(Affinity::Before),
                        head: boundary(Affinity::After),
                    }),
                    history_policy: HistoryPolicy::Skip,
                })
                .unwrap();
            engine
                .apply_command(
                    108_051,
                    TypedCommand::ToggleMark {
                        mark_type: "bold".into(),
                    },
                )
                .unwrap()
                .unwrap();
            assert!(engine.stored_marks().is_some());
            engine
                .apply_typed_transaction(TypedTransaction {
                    request_id: 108_053,
                    base_document_revision: engine.revision(),
                    origin: TransactionOrigin::LocalInput,
                    operations: vec![TypedOperation::InsertText {
                        at: RevisionedPosition {
                            offset: 1,
                            kind: EditorOffsetKind::Scalar,
                            affinity: Affinity::After,
                        },
                        text: "x".into(),
                        marks: engine.stored_marks().unwrap().to_vec(),
                    }],
                    selection_intent: SelectionIntent::Preserve,
                    history_policy: HistoryPolicy::Boundary,
                })
                .unwrap();
            engine
        }

        fn local_state(
            engine: &YrsDocumentEngine,
        ) -> (
            serde_json::Value,
            Option<ResolvedSelection>,
            Option<Vec<crate::model::Mark>>,
            bool,
            bool,
        ) {
            (
                engine.document_json().unwrap(),
                engine.resolved_selection().cloned(),
                engine.stored_marks().map(<[_]>::to_vec),
                engine.can_undo(),
                engine.can_redo(),
            )
        }

        fn text_affinities(engine: &YrsDocumentEngine) -> (Affinity, Affinity) {
            let Some(crate::yrs_engine::RelativeSelection::Text { anchor, head }) =
                engine.relative_selection()
            else {
                panic!("history restores the captured text selection");
            };
            (anchor.affinity, head.affinity)
        }

        let mut fast = fixture();
        let mut fallback = fixture();

        crate::yrs_engine::observability::reset_full_pass_counts_for_test();
        fast.undo_with_result(108_054).unwrap().unwrap();
        let fast_undo_passes = crate::yrs_engine::observability::take_full_pass_counts_for_test();
        let fallback_undo_passes = {
            let _fallback = force_history_document_snapshot_fallback_for_test();
            crate::yrs_engine::observability::reset_full_pass_counts_for_test();
            fallback.undo_with_result(108_054).unwrap().unwrap();
            crate::yrs_engine::observability::take_full_pass_counts_for_test()
        };
        assert_eq!(local_state(&fast), local_state(&fallback));
        assert_eq!(text_affinities(&fast), text_affinities(&fallback));
        assert_eq!(text_affinities(&fast), (Affinity::Before, Affinity::After));
        assert_eq!(fast_undo_passes.canonical_projections, 0);
        assert!(fallback_undo_passes.canonical_projections > 0);

        crate::yrs_engine::observability::reset_full_pass_counts_for_test();
        fast.redo_with_result(108_055).unwrap().unwrap();
        let fast_redo_passes = crate::yrs_engine::observability::take_full_pass_counts_for_test();
        let fallback_redo_passes = {
            let _fallback = force_history_document_snapshot_fallback_for_test();
            crate::yrs_engine::observability::reset_full_pass_counts_for_test();
            fallback.redo_with_result(108_055).unwrap().unwrap();
            crate::yrs_engine::observability::take_full_pass_counts_for_test()
        };
        assert_eq!(local_state(&fast), local_state(&fallback));
        assert_eq!(text_affinities(&fast), text_affinities(&fallback));
        assert_eq!(text_affinities(&fast), (Affinity::Before, Affinity::After));
        assert_eq!(fast_redo_passes.canonical_projections, 0);
        assert!(fallback_redo_passes.canonical_projections > 0);
    }

    #[test]
    fn history_snapshot_context_drift_falls_back_without_changing_undo_result() {
        fn fixture() -> YrsDocumentEngine {
            let mut engine = transaction_engine();
            engine
                .import_json(
                    r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"ab"}]}]}"#,
                    TransactionOrigin::DocumentImport,
                )
                .unwrap();
            engine
                .apply_typed_transaction(TypedTransaction {
                    request_id: 108_060,
                    base_document_revision: engine.revision(),
                    origin: TransactionOrigin::LocalInput,
                    operations: vec![TypedOperation::InsertText {
                        at: RevisionedPosition {
                            offset: 2,
                            kind: EditorOffsetKind::Scalar,
                            affinity: Affinity::After,
                        },
                        text: "x".into(),
                        marks: vec![],
                    }],
                    selection_intent: SelectionIntent::UseOperationResult,
                    history_policy: HistoryPolicy::Boundary,
                })
                .unwrap();
            engine
        }

        for context in ["resource", "editing", "maxLength", "scope"] {
            let mut engine = fixture();
            match context {
                "resource" => {
                    engine.resource_limits.max_document_depth =
                        engine.resource_limits.max_document_depth.saturating_add(1)
                }
                "editing" => {
                    engine.editing_limits.max_operations_per_transaction = engine
                        .editing_limits
                        .max_operations_per_transaction
                        .saturating_add(1)
                }
                "maxLength" => engine.max_length = Some(100),
                "scope" => engine
                    .scope
                    .as_mut()
                    .expect("fixture is document scoped")
                    .lineage_id
                    .push_str("-changed"),
                _ => unreachable!(),
            }

            crate::yrs_engine::observability::reset_full_pass_counts_for_test();
            engine.undo_with_result(108_061).unwrap().unwrap();
            let passes = crate::yrs_engine::observability::take_full_pass_counts_for_test();
            assert_eq!(engine.document().unwrap().root().text_content(), "ab");
            assert!(
                passes.canonical_projections > 0,
                "{context} drift must reject snapshot reuse and run the fallback"
            );
        }
    }

    #[test]
    fn invalid_history_stored_marks_precede_snapshot_publication_and_preserve_atomicity() {
        use crate::yrs_engine::mutation::{
            set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
        };

        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"ab"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 108_070,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: RevisionedPosition {
                        offset: 2,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    text: "x".into(),
                    marks: vec![],
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Boundary,
            })
            .unwrap();
        engine
            .history
            .replace_next_undo_stored_marks_for_test(vec![Mark::new(
                "unknown".into(),
                HashMap::new(),
            )]);
        let before = atomic_audit(&engine);

        set_lookup_seed_hydration_failpoint_for_test(Some(
            LookupSeedHydrationFailpoint::BindingPublication,
        ));
        let result = engine.undo_with_result(108_071);
        set_lookup_seed_hydration_failpoint_for_test(None);

        let error = result.expect_err("invalid history metadata must precede snapshot publication");
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
        assert_eq!(error.request_id, 108_071);
        assert_eq!(
            error.message.as_ref(),
            "history metadata contains invalid stored marks: unknown mark 'unknown'"
        );
        assert_eq!(atomic_audit(&engine), before);
    }

    #[test]
    fn every_history_snapshot_semantic_fallback_precedes_seed_publication() {
        use crate::yrs_engine::derived_state::{
            force_history_document_snapshot_fallback_for_test,
            force_history_snapshot_semantic_fallback_for_test,
            HistorySnapshotSemanticFallbackForTest,
        };
        use crate::yrs_engine::mutation::{
            set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
        };

        fn fixture() -> YrsDocumentEngine {
            let mut engine = transaction_engine();
            engine
                .import_json(
                    r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"ab"}]}]}"#,
                    TransactionOrigin::DocumentImport,
                )
                .unwrap();
            engine
                .apply_typed_transaction(TypedTransaction {
                    request_id: 108_072,
                    base_document_revision: engine.revision(),
                    origin: TransactionOrigin::LocalInput,
                    operations: vec![TypedOperation::InsertText {
                        at: RevisionedPosition {
                            offset: 2,
                            kind: EditorOffsetKind::Scalar,
                            affinity: Affinity::After,
                        },
                        text: "x".into(),
                        marks: vec![],
                    }],
                    selection_intent: SelectionIntent::UseOperationResult,
                    history_policy: HistoryPolicy::Boundary,
                })
                .unwrap();
            engine
        }

        for stage in [
            HistorySnapshotSemanticFallbackForTest::RenderIdentity,
            HistorySnapshotSemanticFallbackForTest::RelativeSelection,
            HistorySnapshotSemanticFallbackForTest::ResolvedSelection,
            HistorySnapshotSemanticFallbackForTest::ResolvedMismatch,
        ] {
            for failpoint in [
                LookupSeedHydrationFailpoint::BindingPublication,
                LookupSeedHydrationFailpoint::SeedPublication,
            ] {
                let mut expected = fixture();
                let expected_result = {
                    let _fallback = force_history_document_snapshot_fallback_for_test();
                    expected.undo_with_result(108_073).unwrap().unwrap()
                };
                let mut actual = fixture();
                let actual_result = {
                    let _fallback = force_history_snapshot_semantic_fallback_for_test(stage);
                    set_lookup_seed_hydration_failpoint_for_test(Some(failpoint));
                    let result = actual.undo_with_result(108_073);
                    set_lookup_seed_hydration_failpoint_for_test(None);
                    result.unwrap().unwrap()
                };

                assert_eq!(actual_result, expected_result, "{stage:?}/{failpoint:?}");
                assert_eq!(
                    actual.document_json(),
                    expected.document_json(),
                    "{stage:?}/{failpoint:?}"
                );
                assert_eq!(
                    actual.resolved_selection(),
                    expected.resolved_selection(),
                    "{stage:?}/{failpoint:?}"
                );
                assert_eq!(
                    actual.stored_marks(),
                    expected.stored_marks(),
                    "{stage:?}/{failpoint:?}"
                );
                assert_eq!(
                    actual.can_undo(),
                    expected.can_undo(),
                    "{stage:?}/{failpoint:?}"
                );
                assert_eq!(
                    actual.can_redo(),
                    expected.can_redo(),
                    "{stage:?}/{failpoint:?}"
                );
            }
        }
    }

    #[test]
    fn history_restore_request_relabeling_precedes_forced_semantic_fallback_and_probes() {
        use crate::yrs_engine::derived_state::{
            force_history_snapshot_semantic_fallback_for_test,
            history_document_snapshot_retained_bytes, DerivedStateCache,
            HistoryDocumentSnapshotRetainedInput, HistorySnapshotSemanticFallbackForTest,
        };
        use crate::yrs_engine::mutation::{
            set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
        };

        let engine = transaction_engine();
        let state = engine.derived_state.as_ref().unwrap();
        let retained =
            history_document_snapshot_retained_bytes(HistoryDocumentSnapshotRetainedInput {
                document: &state.document,
                canonical_artifact: &state.canonical_artifact,
                position_map: &state.position_map,
                rendered_text: &state.rendered_text,
                render_blocks: &state.render_blocks,
                schema_fingerprint: &state.schema_fingerprint,
                fragment_name: &engine.fragment_name,
                scope: engine.scope.as_ref(),
            })
            .unwrap();
        let snapshot = state.capture_history_document_snapshot(
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            &engine.fragment_name,
            engine.scope.as_ref(),
            retained,
        );
        let txn = engine.doc.transact();
        let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();

        for failpoint in [
            LookupSeedHydrationFailpoint::BindingPublication,
            LookupSeedHydrationFailpoint::SeedPublication,
        ] {
            let (_, admission) = snapshot
                .prepare_candidate_read(
                    108_074,
                    &txn,
                    &fragment,
                    &engine.schema,
                    &engine.resource_limits,
                    &engine.editing_limits,
                    engine.max_length,
                    &engine.schema_fingerprint,
                    &engine.fragment_name,
                    engine.scope.as_ref(),
                    engine.yrs_state_epoch,
                    engine.revision,
                )
                .unwrap()
                .into_parts();
            let _fallback = force_history_snapshot_semantic_fallback_for_test(
                HistorySnapshotSemanticFallbackForTest::RenderIdentity,
            );
            set_lookup_seed_hydration_failpoint_for_test(Some(failpoint));
            let result = DerivedStateCache::restore_history_document_snapshot(
                108_075,
                &snapshot,
                admission.expect("matching read admits the retained snapshot"),
                &txn,
                &fragment,
                &engine.schema,
                &state.relative_selection,
                &state.resolved_selection,
                state.stored_marks.clone(),
                &engine.resource_limits,
                &engine.editing_limits,
                engine.max_length,
                &engine.schema_fingerprint,
                engine.revision,
                engine.state_revision,
                engine.yrs_state_epoch,
            );
            set_lookup_seed_hydration_failpoint_for_test(None);

            let error = result.expect_err("request relabeling must precede semantic fallback");
            assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{failpoint:?}");
            assert_eq!(error.request_id, 108_075, "{failpoint:?}");
        }
    }

    #[test]
    fn history_specific_initialization_keeps_candidate_limit_rejection_atomic() {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        engine.max_length = Some(2);
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 108_004,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::DeleteRange {
                    range: RevisionedRange {
                        from: RevisionedPosition {
                            offset: 2,
                            kind: EditorOffsetKind::Scalar,
                            affinity: Affinity::After,
                        },
                        to: RevisionedPosition {
                            offset: 3,
                            kind: EditorOffsetKind::Scalar,
                            affinity: Affinity::After,
                        },
                    },
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Boundary,
            })
            .unwrap();
        let before = atomic_audit(&engine);

        let error = engine.undo_with_result(108_005).unwrap_err();

        assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
        assert_eq!(error.limit, Some(2));
        assert_eq!(error.actual, Some(3));
        assert_eq!(atomic_audit(&engine), before);
    }

    #[test]
    fn second_history_pop_max_length_drift_rejects_before_live_pop() {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"ab"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        engine.max_length = Some(1);
        for (request_id, from, to) in [(108_006, 1, 2), (108_007, 0, 1)] {
            engine
                .apply_typed_transaction(TypedTransaction {
                    request_id,
                    base_document_revision: engine.revision(),
                    origin: TransactionOrigin::LocalInput,
                    operations: vec![TypedOperation::DeleteRange {
                        range: RevisionedRange {
                            from: RevisionedPosition {
                                offset: from,
                                kind: EditorOffsetKind::Scalar,
                                affinity: Affinity::After,
                            },
                            to: RevisionedPosition {
                                offset: to,
                                kind: EditorOffsetKind::Scalar,
                                affinity: Affinity::After,
                            },
                        },
                    }],
                    selection_intent: SelectionIntent::UseOperationResult,
                    history_policy: HistoryPolicy::Boundary,
                })
                .unwrap();
        }

        engine
            .undo(108_008)
            .unwrap()
            .expect("first pop must restore the one-character document");
        assert_eq!(engine.document().unwrap().root().text_content(), "a");
        let before = atomic_audit(&engine);

        let error = engine.undo(108_009).unwrap_err();

        assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
        assert_eq!(error.limit, Some(1));
        assert_eq!(error.actual, Some(2));
        assert_eq!(error.details, Some(json!({ "field": "maxLength" })));
        assert_eq!(atomic_audit(&engine), before);
        let repeated = engine.undo(108_010).unwrap_err();
        assert_eq!(repeated.code, error.code);
        assert_eq!(repeated.limit, error.limit);
        assert_eq!(repeated.actual, error.actual);
        assert_eq!(repeated.details, error.details);
        assert_eq!(atomic_audit(&engine), before);
    }

    #[test]
    fn cached_render_preparation_failure_is_atomic_before_durable_write() {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let before = atomic_audit(&engine);
        crate::render::incremental::set_cached_render_error_for_test(Some(
            crate::render::incremental::CachedRenderError::AllocationFailed,
        ));
        let error = engine
            .apply_typed_transaction(insert_transaction(&engine, 109))
            .unwrap_err();
        crate::render::incremental::set_cached_render_error_for_test(None);

        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
        assert_eq!(atomic_audit(&engine), before);
    }

    fn apply_render_update_for_test(
        mut old_blocks: Vec<Vec<crate::render::RenderElement>>,
        update: crate::yrs_engine::RenderUpdate,
    ) -> Vec<Vec<crate::render::RenderElement>> {
        match update {
            crate::yrs_engine::RenderUpdate::None => old_blocks,
            crate::yrs_engine::RenderUpdate::Full(blocks) => blocks,
            crate::yrs_engine::RenderUpdate::Patch(patch) => {
                old_blocks.splice(
                    patch.start_index..patch.start_index + patch.delete_count,
                    patch.blocks,
                );
                old_blocks
            }
        }
    }

    #[test]
    fn direct_command_admission_error_is_not_replanned_as_structure() {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"target"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        select_text(&mut engine, 104, 6, 0);
        engine.resource_limits.max_input_bytes = 0;
        let before = atomic_audit(&engine);

        let error = engine
            .plan_command(105, TypedCommand::ReplaceSelectionText { text: "x".into() })
            .unwrap_err();

        assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED");
        assert_eq!(error.details, Some(json!({ "field": "maxInputBytes" })));
        assert_eq!(atomic_audit(&engine), before);
    }

    #[test]
    fn every_recoverable_atomic_stage_failpoint_is_pre_open_and_read_only() {
        use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};

        let failpoints = [
            AtomicFailpoint::EnvelopeAdmission,
            AtomicFailpoint::SemanticCompilation,
            AtomicFailpoint::MutationPreflight,
            AtomicFailpoint::FinalPreflight,
            AtomicFailpoint::EncodedAdmission,
            AtomicFailpoint::CanonicalOutputAdmission,
            AtomicFailpoint::RevisionAdmission,
            AtomicFailpoint::DurableMetadataAdmission,
        ];
        for failpoint in failpoints {
            let mut engine = transaction_engine();
            let transaction = insert_transaction(&engine, 76);
            let before = atomic_audit(&engine);
            let canonical_before = engine
                .derived_state
                .as_ref()
                .unwrap()
                .canonical_artifact
                .clone();
            set_atomic_failpoint_for_test(Some(failpoint));

            let error = engine.apply_typed_transaction(transaction).unwrap_err();

            set_atomic_failpoint_for_test(None);
            assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{failpoint:?}");
            assert_eq!(
                error.details,
                Some(json!({ "failpoint": failpoint.field_name() })),
                "{failpoint:?}"
            );
            assert_eq!(atomic_audit(&engine), before, "{failpoint:?}");
            assert!(
                canonical_before.ptr_eq(&engine.derived_state.as_ref().unwrap().canonical_artifact)
            );
        }
    }

    #[test]
    fn compiled_history_failure_does_not_publish_candidate_active_state_lifecycle() {
        use crate::yrs_engine::derived_state::{
            reset_active_state_cache_counts_for_test, take_active_state_cache_counts_for_test,
        };

        for pending_install in [true, false] {
            let request_id = if pending_install { 760_010 } else { 760_020 };
            let mut engine = transaction_engine();
            engine
                .import_json(
                    r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                    TransactionOrigin::DocumentImport,
                )
                .unwrap();
            let point = RevisionedPosition {
                offset: 1,
                kind: EditorOffsetKind::Scalar,
                affinity: Affinity::After,
            };
            engine
                .apply_typed_transaction(TypedTransaction {
                    request_id,
                    base_document_revision: engine.revision(),
                    origin: TransactionOrigin::LocalApi,
                    operations: Vec::new(),
                    selection_intent: SelectionIntent::Set(SelectionInput::Text {
                        anchor: point,
                        head: point,
                    }),
                    history_policy: HistoryPolicy::Skip,
                })
                .unwrap();
            engine
                .apply_command(
                    request_id + 1,
                    TypedCommand::InsertText { text: "x".into() },
                )
                .unwrap()
                .unwrap();
            let live_certificate = engine
                .derived_state
                .as_ref()
                .unwrap()
                .active_state_cache_for_test()
                .expect("fixture must retain a live active-state certificate");
            let preparation = std::cell::RefCell::new(None);
            let CommandPlan::Transaction(transaction) = engine
                .plan_command_internal(
                    request_id + 2,
                    TypedCommand::InsertText { text: "y".into() },
                    Some(&preparation),
                )
                .unwrap()
            else {
                panic!("insert command must prepare a transaction")
            };
            let mut compiled = engine
                .compile_prepared_typed_transaction(transaction, preparation.into_inner().unwrap())
                .unwrap();
            assert!(compiled.prepared_active_state_transition.is_some());
            if !pending_install {
                compiled.prepared_active_state_transition = None;
            }
            let before = atomic_audit(&engine);
            reset_active_state_cache_counts_for_test();
            set_compiled_commit_stage_failpoint_for_test(Some(
                CompiledCommitPreparationStage::HistorySnapshotConstruction,
            ));

            let error = engine
                .apply_compiled_transaction(compiled, true)
                .expect_err("late snapshot construction must reject the prepared candidate");

            set_compiled_commit_stage_failpoint_for_test(None);
            assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{pending_install}");
            assert!(
                error.message.contains("historySnapshotConstruction"),
                "{pending_install}"
            );
            let counts = take_active_state_cache_counts_for_test();
            assert_eq!(counts.5, 0, "pending install={pending_install}");
            assert_eq!(counts.6, 0, "pending install={pending_install}");
            assert_eq!(atomic_audit(&engine), before, "{pending_install}");
            assert!(Arc::ptr_eq(
                &live_certificate,
                &engine
                    .derived_state
                    .as_ref()
                    .unwrap()
                    .active_state_cache_for_test()
                    .unwrap(),
            ));
        }
    }

    #[test]
    fn compiled_recorded_history_admission_preserves_live_replay_allocation_on_later_failure() {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        engine.history.compact_replay_event_capacity_for_test();
        let before = atomic_audit(&engine);
        let ledger_before = engine.history.replay_ledger_allocation_audit_for_test();
        let mut transaction = insert_transaction(&engine, 760_030);
        transaction.history_policy = HistoryPolicy::Auto;
        set_compiled_commit_stage_failpoint_for_test(Some(
            CompiledCommitPreparationStage::HistoryUpdateEncoding,
        ));

        let error = engine
            .apply_typed_transaction(transaction)
            .expect_err("candidate update encoding must fail after recorded admission");

        set_compiled_commit_stage_failpoint_for_test(None);
        assert!(error.message.contains("historyUpdateEncoding"));
        assert_eq!(atomic_audit(&engine), before);
        assert_eq!(
            engine.history.replay_ledger_allocation_audit_for_test(),
            ledger_before
        );
    }

    #[test]
    fn compiled_excluded_history_admission_preserves_live_replay_allocation_on_later_failure() {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        engine.history.compact_replay_event_capacity_for_test();
        let before = atomic_audit(&engine);
        let ledger_before = engine.history.replay_ledger_allocation_audit_for_test();
        let transaction = insert_transaction(&engine, 760_040);
        set_compiled_commit_stage_failpoint_for_test(Some(
            CompiledCommitPreparationStage::HistoryUpdateEncoding,
        ));

        let error = engine
            .apply_typed_transaction(transaction)
            .expect_err("candidate update encoding must fail after excluded admission");

        set_compiled_commit_stage_failpoint_for_test(None);
        assert!(error.message.contains("historyUpdateEncoding"));
        assert_eq!(atomic_audit(&engine), before);
        assert_eq!(
            engine.history.replay_ledger_allocation_audit_for_test(),
            ledger_before
        );
    }

    #[test]
    fn compiled_history_admission_error_precedes_candidate_preparation_failure() {
        use crate::yrs_engine::history::set_replay_update_allocation_failure_for_test;

        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let mut transaction = insert_transaction(&engine, 760_020);
        transaction.history_policy = HistoryPolicy::Auto;
        let compiled = engine.compile_typed_transaction(transaction).unwrap();
        let before = atomic_audit(&engine);
        set_replay_update_allocation_failure_for_test(true);
        set_compiled_commit_stage_failpoint_for_test(Some(
            CompiledCommitPreparationStage::HistoryUpdateEncoding,
        ));

        let error = engine
            .apply_compiled_transaction(compiled, true)
            .expect_err("history admission must win error precedence");

        set_replay_update_allocation_failure_for_test(false);
        set_compiled_commit_stage_failpoint_for_test(None);
        assert_eq!(error.code, "OPERATION_RESOURCE_EXHAUSTED");
        assert_eq!(error.details, Some(json!({ "field": "historyReplay" })));
        assert_eq!(atomic_audit(&engine), before);

        let mut lookup_first = transaction_engine();
        lookup_first
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        lookup_first.ensure_mutation_lookup_seed(760_021).unwrap();
        let mut transaction = insert_transaction(&lookup_first, 760_022);
        transaction.history_policy = HistoryPolicy::Auto;
        let compiled = lookup_first.compile_typed_transaction(transaction).unwrap();
        let before = atomic_audit(&lookup_first);
        set_replay_update_allocation_failure_for_test(true);
        set_compiled_commit_stage_failpoint_for_test(Some(
            CompiledCommitPreparationStage::LookupTransition,
        ));

        let error = lookup_first
            .apply_compiled_transaction(compiled, true)
            .expect_err("baseline lookup failure must retain precedence over history admission");

        set_replay_update_allocation_failure_for_test(false);
        set_compiled_commit_stage_failpoint_for_test(None);
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
        assert!(error.message.contains("lookupTransition"));
        assert_eq!(atomic_audit(&lookup_first), before);
    }

    #[test]
    fn compiled_first_structural_mutation_supports_an_empty_configured_root() {
        let schema = crate::schema::Schema::from_json(&json!({
            "nodes": [
                { "name": "doc", "content": "block*", "role": "doc" },
                { "name": "paragraph", "content": "inline*", "group": "block", "role": "textBlock", "htmlTag": "p" },
                { "name": "text", "group": "inline", "role": "text" }
            ],
            "marks": []
        }))
        .unwrap();
        let mut engine = YrsDocumentEngine::new(YrsEngineConfig {
            schema,
            fragment_name: "empty-root".into(),
            initialization_mode: crate::yrs_engine::InitializationMode::LocalEmpty,
            resource_limits: ResourceLimits::default(),
            editing_limits: crate::yrs_engine::EditingLimits::default(),
            max_length: None,
            scope: Some(crate::yrs_engine::DocumentScope {
                document_id: "empty-root-doc".into(),
                lineage_id: "empty-root-lineage".into(),
            }),
        })
        .unwrap();
        let initial_json = engine.document_json().unwrap();
        let initial_encoded = engine.encoded_state().unwrap();
        let initial_revision = engine.revision();
        let initial_state_revision = engine.state_revision();
        let initial_selection = engine.resolved_selection().cloned();
        let initial_history = engine.history.replay_audit_for_test();
        let result = engine
            .apply_typed_transaction_with_result(TypedTransaction {
                request_id: 760_030,
                base_document_revision: initial_revision,
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertNode {
                    at: RevisionedPosition {
                        offset: 0,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    node: crate::model::Node::element(
                        "paragraph".into(),
                        HashMap::new(),
                        crate::model::Fragment::empty(),
                    ),
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Boundary,
            })
            .unwrap();

        let changed_json = engine.document_json().unwrap();
        assert_ne!(engine.encoded_state().unwrap(), initial_encoded);
        assert_eq!(changed_json["type"], "doc");
        assert_eq!(changed_json["content"][0]["type"], "paragraph");
        assert_eq!(engine.revision(), initial_revision + 1);
        assert_eq!(engine.state_revision(), initial_state_revision + 1);
        assert_eq!(result.document_revision, engine.revision());
        assert_eq!(result.state_revision, engine.state_revision());
        assert_eq!(engine.resolved_selection(), Some(&result.selection));
        assert_eq!(result.history_state.can_undo, engine.can_undo());
        assert_eq!(result.history_state.can_redo, engine.can_redo());
        assert!(engine.can_undo());
        assert_ne!(engine.history.replay_audit_for_test(), initial_history);

        let undo = engine
            .undo(760_031)
            .unwrap()
            .expect("insert must be undoable");
        assert!(undo.changed);
        assert_eq!(engine.document_json().unwrap(), initial_json);
        assert_eq!(engine.resolved_selection(), initial_selection.as_ref());
        assert!(!engine.can_undo());
        assert!(engine.can_redo());
    }

    #[test]
    fn compiled_excluded_rebase_rolls_baseline_and_appends_the_event() {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let before_encoded = engine.encoded_state().unwrap();
        engine.history.force_rebase_before_next_event_for_test();
        let transaction = insert_transaction(&engine, 760_040);

        engine.apply_typed_transaction(transaction).unwrap();

        let (rebase, baseline, event_count, last_is_excluded) =
            engine.history.compiled_excluded_rebase_audit_for_test();
        assert!(!rebase);
        assert_eq!(baseline, before_encoded);
        assert_eq!(event_count, 1);
        assert!(last_is_excluded);
    }

    #[test]
    fn compiled_commit_guard_rejects_every_preparation_stage_after_durable_open() {
        let stages = [
            CompiledCommitPreparationStage::AllocationProbe,
            CompiledCommitPreparationStage::OperationPreparation,
            CompiledCommitPreparationStage::DocumentValidation,
            CompiledCommitPreparationStage::LookupTransition,
            CompiledCommitPreparationStage::HistoryReservation,
            CompiledCommitPreparationStage::HistoryUpdateEncoding,
            CompiledCommitPreparationStage::SelectionFinalization,
            CompiledCommitPreparationStage::DerivedStateBuild,
            CompiledCommitPreparationStage::HistorySnapshotConstruction,
        ];
        for stage in stages {
            set_compiled_commit_stage_failpoint_for_test(None);
            mark_compiled_commit_durable_write_for_test();
            let error = check_compiled_commit_preparation_stage_for_test(760_050, stage)
                .expect_err("every guarded preparation stage must reject after durable open");
            assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{stage:?}");
            assert!(error.message.contains("postwrite"), "{stage:?}");
        }
        set_compiled_commit_stage_failpoint_for_test(None);
    }

    #[test]
    fn compiled_commit_prepares_all_recoverable_work_before_durable_write() {
        let stages = [
            CompiledCommitPreparationStage::AllocationProbe,
            CompiledCommitPreparationStage::OperationPreparation,
            CompiledCommitPreparationStage::DocumentValidation,
            CompiledCommitPreparationStage::LookupTransition,
            CompiledCommitPreparationStage::HistoryReservation,
            CompiledCommitPreparationStage::HistoryUpdateEncoding,
            CompiledCommitPreparationStage::SelectionFinalization,
            CompiledCommitPreparationStage::DerivedStateBuild,
            CompiledCommitPreparationStage::HistorySnapshotConstruction,
        ];
        for (index, stage) in stages.into_iter().enumerate() {
            let mut engine = transaction_engine();
            let request_id = 760_100 + index as u64;
            engine
                .import_json(
                    r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                    TransactionOrigin::DocumentImport,
                )
                .unwrap();
            engine.ensure_mutation_lookup_seed(request_id).unwrap();
            let mut transaction = insert_transaction(&engine, request_id);
            transaction.history_policy = HistoryPolicy::Auto;
            let before = atomic_audit(&engine);
            let seed_before = engine
                .derived_state
                .as_ref()
                .expect("ready fixture has derived state")
                .mutation_lookup_seed
                .clone();
            set_compiled_commit_stage_failpoint_for_test(Some(stage));

            let error = engine
                .apply_typed_transaction(transaction)
                .expect_err("every recoverable compiled-commit stage must be injectable");

            set_compiled_commit_stage_failpoint_for_test(None);
            assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{stage:?}");
            assert_eq!(atomic_audit(&engine), before, "{stage:?}");
            assert!(Arc::ptr_eq(
                &seed_before,
                &engine
                    .derived_state
                    .as_ref()
                    .expect("failed commit retains derived state")
                    .mutation_lookup_seed,
            ));
        }
    }

    #[test]
    fn localized_seed_promotion_is_not_installed_before_any_recoverable_failpoint() {
        use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};
        use crate::yrs_engine::mutation::{
            reset_localized_lookup_counts_for_test, take_localized_lookup_counts_for_test,
        };

        let failpoints = [
            AtomicFailpoint::EnvelopeAdmission,
            AtomicFailpoint::SemanticCompilation,
            AtomicFailpoint::MutationPreflight,
            AtomicFailpoint::FinalPreflight,
            AtomicFailpoint::EncodedAdmission,
            AtomicFailpoint::CanonicalOutputAdmission,
            AtomicFailpoint::RevisionAdmission,
            AtomicFailpoint::DurableMetadataAdmission,
        ];
        for failpoint in failpoints {
            let mut engine = transaction_engine();
            engine
                .import_json(
                    r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"a😀b"}]}]}"#,
                    TransactionOrigin::DocumentImport,
                )
                .unwrap();
            let transaction = insert_transaction(&engine, 76_001);
            let before = atomic_audit(&engine);
            reset_localized_lookup_counts_for_test();
            set_atomic_failpoint_for_test(Some(failpoint));

            let error = engine.apply_typed_transaction(transaction).unwrap_err();

            set_atomic_failpoint_for_test(None);
            let (_, _, promotions) = take_localized_lookup_counts_for_test();
            assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{failpoint:?}");
            assert_eq!(promotions, 0, "{failpoint:?}");
            assert_eq!(atomic_audit(&engine), before, "{failpoint:?}");
        }
    }

    #[test]
    fn empty_skip_selection_bypasses_mutation_preflight_but_not_admission_or_boundaries() {
        use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};
        use crate::yrs_engine::observability::{
            reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
        };

        let selection_transaction =
            |engine: &YrsDocumentEngine, request_id, history_policy| TypedTransaction {
                request_id,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: vec![],
                selection_intent: SelectionIntent::Set(SelectionInput::All),
                history_policy,
            };

        let mut skip = transaction_engine();
        reset_prepared_admission_counts_for_test();
        set_atomic_failpoint_for_test(Some(AtomicFailpoint::MutationPreflight));
        let result = skip
            .apply_typed_transaction_with_result(selection_transaction(
                &skip,
                760,
                HistoryPolicy::Skip,
            ))
            .expect("empty Skip selection must not enter mutation preflight");
        set_atomic_failpoint_for_test(None);
        assert!(result.changed);
        assert_eq!(skip.revision(), 0);
        assert_eq!(skip.state_revision(), 1);
        let skip_counts = take_prepared_admission_counts_for_test();
        assert_eq!(skip_counts.staged_seed_preparations, 0);
        assert_eq!(skip_counts.installed_base_seed_publications, 0);

        let mut boundary = transaction_engine();
        let before_boundary = atomic_audit(&boundary);
        set_atomic_failpoint_for_test(Some(AtomicFailpoint::MutationPreflight));
        let boundary_error = boundary
            .apply_typed_transaction(selection_transaction(
                &boundary,
                761,
                HistoryPolicy::Boundary,
            ))
            .unwrap_err();
        set_atomic_failpoint_for_test(None);
        assert_eq!(
            boundary_error.details,
            Some(json!({ "failpoint": "mutationPreflight" }))
        );
        assert_eq!(atomic_audit(&boundary), before_boundary);

        let mut rejected = transaction_engine();
        let before_rejected = atomic_audit(&rejected);
        set_atomic_failpoint_for_test(Some(AtomicFailpoint::EnvelopeAdmission));
        let admission_error = rejected
            .apply_typed_transaction(selection_transaction(&rejected, 762, HistoryPolicy::Skip))
            .unwrap_err();
        set_atomic_failpoint_for_test(None);
        assert_eq!(
            admission_error.details,
            Some(json!({ "failpoint": "envelopeAdmission" }))
        );
        assert_eq!(atomic_audit(&rejected), before_rejected);
    }

    #[test]
    fn empty_generic_state_only_transactions_do_not_prepare_lookup_seed() {
        use crate::yrs_engine::mutation::{
            set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
        };
        use crate::yrs_engine::observability::{
            reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
        };

        for (offset, history_policy) in [
            HistoryPolicy::Skip,
            HistoryPolicy::Auto,
            HistoryPolicy::Boundary,
        ]
        .into_iter()
        .enumerate()
        {
            let request_id = 760_100 + u64::try_from(offset).unwrap();
            let mut engine = import_document_with_unavailable_lookup_seed();
            engine
                .apply_command(
                    request_id,
                    TypedCommand::ToggleMark {
                        mark_type: "bold".into(),
                    },
                )
                .unwrap()
                .expect("collapsed toggle must set stored marks");
            assert_eq!(
                engine
                    .stored_marks()
                    .unwrap()
                    .iter()
                    .map(Mark::mark_type)
                    .collect::<Vec<_>>(),
                vec!["bold"]
            );
            let installed =
                Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
            let before_document_revision = engine.revision();
            let before_state_revision = engine.state_revision();
            reset_prepared_admission_counts_for_test();
            set_lookup_seed_hydration_failpoint_for_test(Some(
                LookupSeedHydrationFailpoint::InitialReservation,
            ));

            let result = engine
                .apply_typed_transaction_with_result(TypedTransaction {
                    request_id: request_id + 10,
                    base_document_revision: engine.revision(),
                    origin: TransactionOrigin::LocalApi,
                    operations: Vec::new(),
                    selection_intent: SelectionIntent::Set(SelectionInput::All),
                    history_policy,
                })
                .expect("state-only generic transaction must not consume hydration failure");

            set_lookup_seed_hydration_failpoint_for_test(None);
            let counts = take_prepared_admission_counts_for_test();
            assert!(result.changed, "{history_policy:?}");
            assert_eq!(
                result.selection,
                ResolvedSelection::All,
                "{history_policy:?}"
            );
            assert_eq!(
                engine.revision(),
                before_document_revision,
                "{history_policy:?}"
            );
            assert_eq!(
                engine.state_revision(),
                before_state_revision + 1,
                "{history_policy:?}"
            );
            assert!(engine.stored_marks().is_none(), "{history_policy:?}");
            assert_eq!(counts.staged_seed_preparations, 0, "{history_policy:?}");
            assert_eq!(
                counts.installed_base_seed_publications, 0,
                "{history_policy:?}"
            );
            assert!(Arc::ptr_eq(
                &installed,
                &engine.derived_state.as_ref().unwrap().mutation_lookup_seed,
            ));
            assert!(engine
                .derived_state
                .as_ref()
                .unwrap()
                .mutation_lookup_seed
                .is_unavailable_for_test());
        }
    }

    #[test]
    fn empty_generic_boundary_preserves_recorded_grouping_semantics() {
        let apply_insert = |engine: &mut YrsDocumentEngine, request_id, text: &str| {
            let at = engine.position_map().unwrap().total_scalars();
            engine
                .apply_typed_transaction(TypedTransaction {
                    request_id,
                    base_document_revision: engine.revision(),
                    origin: TransactionOrigin::LocalInput,
                    operations: vec![TypedOperation::InsertText {
                        at: RevisionedPosition {
                            offset: at,
                            kind: EditorOffsetKind::Scalar,
                            affinity: Affinity::After,
                        },
                        text: text.into(),
                        marks: Vec::new(),
                    }],
                    selection_intent: SelectionIntent::Preserve,
                    history_policy: HistoryPolicy::Auto,
                })
                .unwrap();
        };

        for (offset, state_only_policy) in [HistoryPolicy::Auto, HistoryPolicy::Boundary]
            .into_iter()
            .enumerate()
        {
            let request_id = 760_120 + u64::try_from(offset).unwrap() * 10;
            let mut engine = import_document_with_unavailable_lookup_seed();
            apply_insert(&mut engine, request_id, "x");
            force_lookup_seed_unavailable(&mut engine);
            engine
                .apply_typed_transaction(TypedTransaction {
                    request_id: request_id + 1,
                    base_document_revision: engine.revision(),
                    origin: TransactionOrigin::LocalApi,
                    operations: Vec::new(),
                    selection_intent: SelectionIntent::Set(SelectionInput::All),
                    history_policy: state_only_policy,
                })
                .unwrap();
            apply_insert(&mut engine, request_id + 2, "y");
            assert_eq!(engine.document().unwrap().root().text_content(), "abcxy");

            engine
                .undo(request_id + 3)
                .unwrap()
                .expect("recorded insert must be undoable");
            let expected_after_first_pop = if state_only_policy == HistoryPolicy::Boundary {
                "abcx"
            } else {
                "abc"
            };
            assert_eq!(
                engine.document().unwrap().root().text_content(),
                expected_after_first_pop,
                "{state_only_policy:?}"
            );
            if state_only_policy == HistoryPolicy::Boundary {
                engine
                    .undo(request_id + 4)
                    .unwrap()
                    .expect("Boundary must retain the earlier group");
                assert_eq!(engine.document().unwrap().root().text_content(), "abc");
            }
        }
    }

    #[test]
    fn changed_state_boundary_revision_overflow_precedes_replay_allocation() {
        let mut engine = import_document_with_unavailable_lookup_seed();
        engine.history.compact_replay_event_capacity_for_test();
        engine.state_revision = u64::MAX;
        engine
            .derived_state
            .as_mut()
            .unwrap()
            .reseal_state_revision(u64::MAX);
        let before = atomic_audit(&engine);
        let replay_before = engine.history.replay_ledger_allocation_audit_for_test();

        let error = engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 760_110,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: Vec::new(),
                selection_intent: SelectionIntent::Set(SelectionInput::All),
                history_policy: HistoryPolicy::Boundary,
            })
            .unwrap_err();

        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{error:?}");
        assert_eq!(
            error.message.as_ref(),
            "stateRevision cannot be incremented"
        );
        assert_eq!(error.details, Some(json!({ "field": "stateRevision" })));
        assert_eq!(atomic_audit(&engine), before);
        assert_eq!(
            engine.history.replay_ledger_allocation_audit_for_test(),
            replay_before
        );
    }

    #[test]
    fn generic_structural_limit_drift_matches_preconfigured_current_and_reuses_evidence() {
        let source = r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"a"}]}]}"#;
        let schema = tiptap_schema();
        let base_document = from_prosemirror_json(
            &serde_json::from_str(source).unwrap(),
            &schema,
            UnknownTypeMode::Preserve,
        )
        .unwrap();
        let old_node_limit = crate::editor_state::document_node_count(base_document.root());
        let current_node_limit = old_node_limit + 1;
        let old_limits = ResourceLimits {
            max_document_nodes: old_node_limit,
            ..ResourceLimits::default()
        };
        let current_limits = ResourceLimits {
            max_document_nodes: current_node_limit,
            ..old_limits.clone()
        };

        let mut drifted = transaction_engine_with_resource_limits_and_mode(
            old_limits.clone(),
            crate::yrs_engine::InitializationMode::LocalEmpty,
        );
        let mut preconfigured = transaction_engine_with_resource_limits_and_mode(
            current_limits.clone(),
            crate::yrs_engine::InitializationMode::LocalEmpty,
        );
        let mut one_under = transaction_engine_with_resource_limits_and_mode(
            current_limits.clone(),
            crate::yrs_engine::InitializationMode::LocalEmpty,
        );
        for engine in [&mut drifted, &mut preconfigured, &mut one_under] {
            engine
                .import_json(source, TransactionOrigin::DocumentImport)
                .unwrap();
        }
        assert_eq!(
            drifted.derived_state.as_ref().unwrap().document_node_count,
            old_node_limit
        );
        assert!(derived_evidence_matches_runtime_limits(&drifted));
        drifted.resource_limits = current_limits.clone();
        assert!(!derived_evidence_matches_runtime_limits(&drifted));

        let drifted_commit = drifted
            .apply_typed_transaction(hard_break_insert_transaction(&drifted, 760_200))
            .expect("loosened runtime limit must admit the generic structural candidate");
        let preconfigured_commit = preconfigured
            .apply_typed_transaction(hard_break_insert_transaction(&preconfigured, 760_200))
            .unwrap();
        assert_eq!(drifted_commit, preconfigured_commit);
        assert_eq!(drifted_commit.document_revision, 2);
        assert_eq!(drifted_commit.state_revision, 2);
        assert_eq!(
            drifted.derived_state.as_ref().unwrap().document_node_count,
            current_node_limit
        );
        assert_limit_drift_semantic_parity(&drifted, &preconfigured);

        let drifted_followup = drifted
            .apply_typed_transaction(insert_transaction(&drifted, 760_201))
            .expect("current-limit evidence must be reusable by the following mutation");
        let preconfigured_followup = preconfigured
            .apply_typed_transaction(insert_transaction(&preconfigured, 760_201))
            .unwrap();
        assert_eq!(drifted_followup, preconfigured_followup);
        assert_limit_drift_semantic_parity(&drifted, &preconfigured);

        one_under.resource_limits = old_limits;
        let before = atomic_audit(&one_under);
        let error = one_under
            .apply_typed_transaction(hard_break_insert_transaction(&one_under, 760_202))
            .unwrap_err();
        assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
        assert_eq!(error.limit, Some(u64::try_from(old_node_limit).unwrap()));
        assert_eq!(
            error.actual,
            Some(u64::try_from(current_node_limit).unwrap())
        );
        assert_eq!(atomic_audit(&one_under), before);
    }

    #[test]
    fn remote_limit_drift_matches_preconfigured_current_and_reuses_evidence() {
        let source_json = r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"a"}]}]}"#;
        let schema = tiptap_schema();
        let base_document = from_prosemirror_json(
            &serde_json::from_str(source_json).unwrap(),
            &schema,
            UnknownTypeMode::Preserve,
        )
        .unwrap();
        let old_node_limit = crate::editor_state::document_node_count(base_document.root());
        let current_node_limit = old_node_limit + 1;
        let old_limits = ResourceLimits {
            max_document_nodes: old_node_limit,
            ..ResourceLimits::default()
        };
        let current_limits = ResourceLimits {
            max_document_nodes: current_node_limit,
            ..old_limits.clone()
        };
        let mut source = transaction_engine_with_resource_limits_and_mode(
            current_limits.clone(),
            crate::yrs_engine::InitializationMode::LocalEmpty,
        );
        source
            .import_json(source_json, TransactionOrigin::DocumentImport)
            .unwrap();
        let base_update = source.encoded_state().unwrap();
        let mut drifted = transaction_engine_with_resource_limits_and_mode(
            old_limits,
            crate::yrs_engine::InitializationMode::AwaitRemote,
        );
        let mut preconfigured = transaction_engine_with_resource_limits_and_mode(
            current_limits.clone(),
            crate::yrs_engine::InitializationMode::AwaitRemote,
        );
        let drifted_base = drifted
            .apply_remote_update_v1(760_210, &base_update)
            .unwrap();
        let preconfigured_base = preconfigured
            .apply_remote_update_v1(760_210, &base_update)
            .unwrap();
        assert_eq!(drifted_base, preconfigured_base);
        assert!(derived_evidence_matches_runtime_limits(&drifted));
        drifted.resource_limits = current_limits;
        assert!(!derived_evidence_matches_runtime_limits(&drifted));

        let target_vector = drifted.doc.transact().state_vector();
        source
            .apply_typed_transaction(paragraph_insert_transaction(&source, 760_211))
            .unwrap();
        let structural_delta = source
            .doc
            .transact()
            .encode_state_as_update_v1(&target_vector);
        let drifted_commit = drifted
            .apply_remote_update_v1(760_212, &structural_delta)
            .expect("loosened runtime limit must admit the changed remote candidate");
        let preconfigured_commit = preconfigured
            .apply_remote_update_v1(760_212, &structural_delta)
            .unwrap();
        assert_eq!(drifted_commit, preconfigured_commit);
        assert_eq!(
            drifted.derived_state.as_ref().unwrap().document_node_count,
            current_node_limit
        );
        assert_limit_drift_semantic_parity(&drifted, &preconfigured);

        let target_vector = drifted.doc.transact().state_vector();
        source
            .apply_typed_transaction(insert_transaction(&source, 760_213))
            .unwrap();
        let followup_delta = source
            .doc
            .transact()
            .encode_state_as_update_v1(&target_vector);
        let drifted_followup = drifted
            .apply_remote_update_v1(760_214, &followup_delta)
            .expect("remote current-limit evidence must be reusable");
        let preconfigured_followup = preconfigured
            .apply_remote_update_v1(760_214, &followup_delta)
            .unwrap();
        assert_eq!(drifted_followup, preconfigured_followup);
        assert_limit_drift_semantic_parity(&drifted, &preconfigured);
    }

    #[test]
    fn empty_skip_collapsed_text_prepares_one_forward_point_without_reverse_traversal() {
        use crate::yrs_engine::derived_state::{
            reset_relative_selection_traversal_counts_for_test,
            take_relative_selection_traversal_counts_for_test,
        };
        use crate::yrs_engine::position::{
            reset_relative_position_traversal_counts_for_test,
            take_relative_position_traversal_counts_for_test,
        };

        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"prefix"}]},{"type":"paragraph","content":[{"type":"text","text":"a😀middle"}]},{"type":"paragraph","content":[{"type":"text","text":"suffix"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let point = RevisionedPosition {
            offset: 9,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::Before,
        };
        reset_relative_position_traversal_counts_for_test();
        reset_relative_selection_traversal_counts_for_test();

        let result = engine
            .apply_typed_transaction_with_result(TypedTransaction {
                request_id: 759,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: vec![],
                selection_intent: SelectionIntent::Set(SelectionInput::Text {
                    anchor: point,
                    head: point,
                }),
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap();

        assert!(result.changed);
        assert_eq!(result.document_revision, 1);
        assert_eq!(result.state_revision, 2);
        assert!(matches!(
            result.selection,
            ResolvedSelection::Text { anchor, head }
                if anchor == head && anchor.scalar == point.offset
        ));
        assert_eq!(
            take_relative_position_traversal_counts_for_test(),
            (0, 1, 0),
            "collapsed exact inputs must share one admitted forward materialization"
        );
        assert_eq!(
            take_relative_selection_traversal_counts_for_test(),
            (0, 0),
            "prepared resolved points must not round-trip through Yrs"
        );
    }

    #[test]
    fn empty_skip_prepared_collapsed_text_preserves_overflow_and_output_atomicity() {
        fn populated_engine() -> YrsDocumentEngine {
            let mut engine = transaction_engine();
            engine
                .import_json(
                    r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abcdef"}]}]}"#,
                    TransactionOrigin::DocumentImport,
                )
                .unwrap();
            engine
        }

        fn transaction(engine: &YrsDocumentEngine, request_id: u64) -> TypedTransaction {
            let point = RevisionedPosition {
                offset: 3,
                kind: EditorOffsetKind::Scalar,
                affinity: Affinity::Before,
            };
            TypedTransaction {
                request_id,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: vec![],
                selection_intent: SelectionIntent::Set(SelectionInput::Text {
                    anchor: point,
                    head: point,
                }),
                history_policy: HistoryPolicy::Skip,
            }
        }

        let mut overflow = populated_engine();
        overflow.state_revision = u64::MAX;
        overflow.derived_state.as_mut().unwrap().state_revision = u64::MAX;
        let overflow_before = atomic_audit(&overflow);
        let overflow_transaction = transaction(&overflow, 759_001);

        let overflow_error = overflow
            .apply_typed_transaction_with_result(overflow_transaction)
            .unwrap_err();

        assert_eq!(overflow_error.code, "ENGINE_INVARIANT_FAILED");
        assert_eq!(
            overflow_error.details,
            Some(json!({ "field": "stateRevision" }))
        );
        assert_eq!(atomic_audit(&overflow), overflow_before);

        let mut output_limited = populated_engine();
        output_limited.editing_limits.max_derived_output_bytes = 1;
        let output_before = atomic_audit(&output_limited);
        let output_transaction = transaction(&output_limited, 759_002);

        let output_error = output_limited
            .apply_typed_transaction_with_result(output_transaction)
            .unwrap_err();

        assert_eq!(output_error.code, "DOCUMENT_LIMIT_EXCEEDED");
        assert_eq!(
            output_error.details,
            Some(json!({ "field": "maxDerivedOutputBytes" }))
        );
        assert_eq!(atomic_audit(&output_limited), output_before);
    }

    #[test]
    fn empty_skip_fast_path_matches_full_compiler_at_yrs_scan_work_boundary() {
        fn populated_engine() -> YrsDocumentEngine {
            let mut engine = transaction_engine();
            engine
                .import_json(
                    r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"scan boundary"}]}]}"#,
                    TransactionOrigin::DocumentImport,
                )
                .unwrap();
            engine
        }

        fn scan_work(engine: &YrsDocumentEngine) -> usize {
            let document_text_bytes = engine.document().unwrap().root().text_content().len();
            let txn = engine.doc.transact();
            let crdt_clock_work = txn
                .state_vector()
                .iter()
                .map(|(_, clock)| usize::try_from(*clock).unwrap() + 1)
                .sum::<usize>();
            document_text_bytes * 2 + crdt_clock_work * 2
        }

        fn transaction(engine: &YrsDocumentEngine, request_id: u64) -> TypedTransaction {
            TypedTransaction {
                request_id,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: vec![],
                selection_intent: SelectionIntent::Set(SelectionInput::All),
                history_policy: HistoryPolicy::Skip,
            }
        }

        let required = scan_work(&populated_engine());

        let mut exact_fast = populated_engine();
        exact_fast.resource_limits.max_input_bytes = required;
        let exact_fast_result = exact_fast
            .apply_typed_transaction_with_result(transaction(&exact_fast, 763))
            .unwrap();
        let mut exact_slow = populated_engine();
        exact_slow.resource_limits.max_input_bytes = required;
        let exact_slow_transaction = transaction(&exact_slow, 763);
        let exact_slow_compiled = exact_slow
            .compile_typed_transaction(exact_slow_transaction)
            .unwrap();
        let exact_slow_result = exact_slow
            .apply_compiled_transaction(exact_slow_compiled, true)
            .unwrap()
            .1
            .unwrap();
        assert_eq!(exact_fast_result, exact_slow_result);
        assert_eq!(exact_fast.document_json(), exact_slow.document_json());
        assert_eq!(exact_fast.document_html(), exact_slow.document_html());
        assert_eq!(exact_fast.revision(), exact_slow.revision());
        assert_eq!(exact_fast.state_revision(), exact_slow.state_revision());
        assert_eq!(
            exact_fast.resolved_selection(),
            exact_slow.resolved_selection()
        );
        assert_eq!(exact_fast.stored_marks(), exact_slow.stored_marks());
        assert_eq!(exact_fast.can_undo(), exact_slow.can_undo());
        assert_eq!(exact_fast.can_redo(), exact_slow.can_redo());

        let mut one_under_slow = populated_engine();
        one_under_slow.resource_limits.max_input_bytes = required - 1;
        let before_slow = atomic_audit(&one_under_slow);
        let slow_error = one_under_slow
            .compile_typed_transaction(transaction(&one_under_slow, 764))
            .unwrap_err();
        assert_eq!(atomic_audit(&one_under_slow), before_slow);

        let mut one_under_fast = populated_engine();
        one_under_fast.resource_limits.max_input_bytes = required - 1;
        let before_fast = atomic_audit(&one_under_fast);
        let fast_error = one_under_fast
            .apply_typed_transaction_with_result(transaction(&one_under_fast, 764))
            .unwrap_err();
        assert_eq!(fast_error, slow_error);
        assert_eq!(atomic_audit(&one_under_fast), before_fast);

        let mut changed_document = populated_engine();
        changed_document
            .apply_command(765, TypedCommand::InsertText { text: "é".into() })
            .unwrap()
            .unwrap();
        let cached_text_bytes = changed_document
            .derived_state
            .as_ref()
            .unwrap()
            .document_text_bytes;
        assert_eq!(
            cached_text_bytes,
            changed_document
                .document()
                .unwrap()
                .root()
                .text_content()
                .len()
        );
        let changed_required = scan_work(&changed_document);
        changed_document.resource_limits.max_input_bytes = changed_required - 1;
        let before_changed = atomic_audit(&changed_document);
        let changed_error = changed_document
            .apply_typed_transaction_with_result(transaction(&changed_document, 766))
            .unwrap_err();
        assert_eq!(changed_error.code, "OPERATION_LIMIT_EXCEEDED");
        assert_eq!(
            changed_error.limit,
            Some(u64::try_from(changed_required - 1).unwrap())
        );
        assert_eq!(
            changed_error.actual,
            Some(u64::try_from(changed_required).unwrap())
        );
        assert_eq!(atomic_audit(&changed_document), before_changed);

        let invalid_selection = |engine: &YrsDocumentEngine| TypedTransaction {
            request_id: 767,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent: SelectionIntent::Set(SelectionInput::Text {
                anchor: RevisionedPosition {
                    offset: u32::MAX,
                    kind: EditorOffsetKind::Utf16,
                    affinity: Affinity::Before,
                },
                head: RevisionedPosition {
                    offset: u32::MAX,
                    kind: EditorOffsetKind::Utf16,
                    affinity: Affinity::After,
                },
            }),
            history_policy: HistoryPolicy::Skip,
        };
        let invalid_slow = populated_engine();
        let before_invalid_slow = atomic_audit(&invalid_slow);
        let invalid_slow_error = invalid_slow
            .compile_typed_transaction(invalid_selection(&invalid_slow))
            .unwrap_err();
        assert_eq!(atomic_audit(&invalid_slow), before_invalid_slow);
        let mut invalid_fast = populated_engine();
        let before_invalid_fast = atomic_audit(&invalid_fast);
        let invalid_fast_error = invalid_fast
            .apply_typed_transaction_with_result(invalid_selection(&invalid_fast))
            .unwrap_err();
        assert_eq!(invalid_fast_error, invalid_slow_error);
        assert_eq!(atomic_audit(&invalid_fast), before_invalid_fast);
    }

    #[test]
    fn empty_skip_fast_path_matches_full_compiler_for_selection_forms_and_local_state() {
        fn populated_engine() -> YrsDocumentEngine {
            let mut engine = transaction_engine();
            engine
                .import_json(
                    &json!({
                        "type": "doc",
                        "content": [
                            {"type": "paragraph", "content": [{"type": "text", "text": "a😀b"}]},
                            {"type": "horizontalRule"},
                            {"type": "paragraph", "content": [{"type": "text", "text": "tail"}]}
                        ]
                    })
                    .to_string(),
                    TransactionOrigin::DocumentImport,
                )
                .unwrap();
            engine
        }

        fn transaction(
            engine: &YrsDocumentEngine,
            request_id: u64,
            selection_intent: SelectionIntent,
        ) -> TypedTransaction {
            TypedTransaction {
                request_id,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: vec![],
                selection_intent,
                history_policy: HistoryPolicy::Skip,
            }
        }

        fn slow_result(
            engine: &mut YrsDocumentEngine,
            transaction: TypedTransaction,
        ) -> crate::yrs_engine::TypedTransactionResult {
            let compiled = engine.compile_typed_transaction(transaction).unwrap();
            engine
                .apply_compiled_transaction(compiled, true)
                .unwrap()
                .1
                .unwrap()
        }

        let scalar = |offset, affinity| RevisionedPosition {
            offset,
            kind: EditorOffsetKind::Scalar,
            affinity,
        };
        let utf16 = |offset, affinity| RevisionedPosition {
            offset,
            kind: EditorOffsetKind::Utf16,
            affinity,
        };
        let intents = [
            SelectionIntent::Set(SelectionInput::Text {
                anchor: scalar(2, Affinity::Before),
                head: scalar(2, Affinity::Before),
            }),
            SelectionIntent::Set(SelectionInput::Text {
                anchor: scalar(2, Affinity::Before),
                head: scalar(2, Affinity::After),
            }),
            SelectionIntent::Set(SelectionInput::Text {
                anchor: utf16(3, Affinity::Before),
                head: utf16(3, Affinity::After),
            }),
            SelectionIntent::Set(SelectionInput::Node {
                at: scalar(4, Affinity::Before),
            }),
            SelectionIntent::Set(SelectionInput::All),
            SelectionIntent::Preserve,
            SelectionIntent::UseOperationResult,
        ];

        for (index, intent) in intents.into_iter().enumerate() {
            let mut fast = populated_engine();
            let mut slow = populated_engine();
            let fast_before = atomic_audit(&fast);
            let slow_before = atomic_audit(&slow);
            let fast_transaction = transaction(&fast, 770 + index as u64, intent.clone());
            let slow_transaction = transaction(&slow, 770 + index as u64, intent.clone());

            let fast_result = fast
                .apply_typed_transaction_with_result(fast_transaction)
                .unwrap();
            let slow_result = slow_result(&mut slow, slow_transaction);

            assert_eq!(fast_result, slow_result, "intent={intent:?}");
            assert_eq!(
                fast.document_json(),
                slow.document_json(),
                "intent={intent:?}"
            );
            assert_eq!(
                fast.document_html(),
                slow.document_html(),
                "intent={intent:?}"
            );
            assert_eq!(fast.revision(), slow.revision(), "intent={intent:?}");
            assert_eq!(
                fast.state_revision(),
                slow.state_revision(),
                "intent={intent:?}"
            );
            assert_eq!(
                fast.resolved_selection(),
                slow.resolved_selection(),
                "intent={intent:?}"
            );
            assert_eq!(
                fast.stored_marks(),
                slow.stored_marks(),
                "intent={intent:?}"
            );
            assert_eq!(fast.can_undo(), slow.can_undo(), "intent={intent:?}");
            assert_eq!(fast.can_redo(), slow.can_redo(), "intent={intent:?}");
            assert_eq!(fast.encoded_state().unwrap(), fast_before.encoded);
            assert_eq!(slow.encoded_state().unwrap(), slow_before.encoded);
            assert_eq!(fast.yrs_state_epoch, fast_before.yrs_state_epoch);
            assert_eq!(slow.yrs_state_epoch, slow_before.yrs_state_epoch);
            assert_eq!(
                fast.history.replay_audit_for_test(),
                fast_before.replay_audit
            );
            assert_eq!(
                slow.history.replay_audit_for_test(),
                slow_before.replay_audit
            );
        }

        let stored_mark_intents = [
            SelectionIntent::Set(SelectionInput::Text {
                anchor: scalar(1, Affinity::Before),
                head: scalar(1, Affinity::Before),
            }),
            SelectionIntent::Set(SelectionInput::Text {
                anchor: scalar(1, Affinity::Before),
                head: scalar(1, Affinity::After),
            }),
            SelectionIntent::Set(SelectionInput::Text {
                anchor: scalar(2, Affinity::Before),
                head: scalar(2, Affinity::Before),
            }),
            SelectionIntent::Set(SelectionInput::Node {
                at: scalar(4, Affinity::Before),
            }),
        ];
        for (index, intent) in stored_mark_intents.into_iter().enumerate() {
            let mut fast = populated_engine();
            let mut slow = populated_engine();
            select_text(&mut fast, 780, 1, 1);
            select_text(&mut slow, 780, 1, 1);
            for engine in [&mut fast, &mut slow] {
                engine
                    .apply_command(
                        781,
                        TypedCommand::ToggleMark {
                            mark_type: "bold".into(),
                        },
                    )
                    .unwrap()
                    .unwrap();
                assert!(engine.stored_marks().is_some());
            }
            let fast_transaction = transaction(&fast, 782 + index as u64, intent.clone());
            let slow_transaction = transaction(&slow, 782 + index as u64, intent.clone());

            let fast_result = fast
                .apply_typed_transaction_with_result(fast_transaction)
                .unwrap();
            let slow_result = slow_result(&mut slow, slow_transaction);

            assert_eq!(fast_result, slow_result, "stored intent={intent:?}");
            assert_eq!(
                fast.resolved_selection(),
                slow.resolved_selection(),
                "stored intent={intent:?}"
            );
            assert_eq!(
                fast.stored_marks(),
                slow.stored_marks(),
                "stored intent={intent:?}"
            );
            if index <= 1 {
                assert!(fast.stored_marks().is_some());
            } else {
                assert!(fast.stored_marks().is_none());
            }
        }
    }

    #[test]
    fn remote_history_admission_failure_retains_dependency_quarantine_for_retry() {
        use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};
        use yrs::{diff_updates_v1, encode_state_vector_from_update_v1};

        let mut source = transaction_engine();
        let base = source.encoded_state().unwrap();
        source
            .apply_command(200, TypedCommand::InsertText { text: "a".into() })
            .unwrap();
        let after_a = source.encoded_state().unwrap();
        source
            .apply_command(201, TypedCommand::InsertText { text: "b".into() })
            .unwrap();
        let after_b = source.encoded_state().unwrap();
        let base_sv = encode_state_vector_from_update_v1(&base).unwrap();
        let after_a_sv = encode_state_vector_from_update_v1(&after_a).unwrap();
        let delta_a = diff_updates_v1(&after_a, &base_sv).unwrap();
        let delta_b = diff_updates_v1(&after_b, &after_a_sv).unwrap();

        let mut target = YrsDocumentEngine::new(YrsEngineConfig {
            schema: tiptap_schema(),
            fragment_name: "prosemirror".into(),
            initialization_mode: crate::yrs_engine::InitializationMode::AwaitRemote,
            resource_limits: ResourceLimits::default(),
            editing_limits: crate::yrs_engine::EditingLimits::default(),
            max_length: None,
            scope: Some(crate::yrs_engine::DocumentScope {
                document_id: "doc".into(),
                lineage_id: "lineage".into(),
            }),
        })
        .unwrap();
        assert!(
            !target
                .apply_remote_update_v1(202, &delta_b)
                .unwrap()
                .changed
        );
        assert!(
            !target
                .apply_remote_update_v1(203, &delta_a)
                .unwrap()
                .changed
        );
        let before = atomic_audit(&target);

        set_atomic_failpoint_for_test(Some(AtomicFailpoint::RemoteHistoryAdmission));
        let error = target.apply_remote_update_v1(204, &base).unwrap_err();
        set_atomic_failpoint_for_test(None);

        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
        assert_eq!(
            error.details,
            Some(json!({ "failpoint": "remoteHistoryAdmission" }))
        );
        assert_eq!(atomic_audit(&target), before);
        let retry = target.apply_remote_update_v1(205, &base).unwrap();
        assert!(retry.changed);
        assert_eq!(target.document().unwrap().root().text_content(), "ab");
        assert_eq!(target.encoded_state().unwrap(), after_b);
    }

    /// Task 9 classification seam: the read-only preflight accepts exactly
    /// what the prepare pipeline's ingress admission accepts, rejects
    /// malformed encodings with the same structured errors, and never
    /// touches engine state.
    #[test]
    fn preflight_remote_update_v1_classifies_encoding_without_engine_effects() {
        let mut source = transaction_engine();
        source
            .apply_command(210, TypedCommand::InsertText { text: "pf".into() })
            .unwrap();
        let valid = source.encoded_state().unwrap();
        let engine = transaction_engine();
        let before = atomic_audit(&engine);

        engine.preflight_remote_update_v1(211, &valid).unwrap();
        engine.preflight_remote_update_v1(212, &[0, 0]).unwrap();

        let error = engine
            .preflight_remote_update_v1(213, &[0xff, 0xff, 0xff])
            .unwrap_err();
        assert_eq!(error.code, "DOCUMENT_INVALID");
        assert_eq!(error.request_id, 213);

        let mut truncated = valid.clone();
        truncated.truncate(valid.len() / 2);
        assert!(engine.preflight_remote_update_v1(214, &truncated).is_err());

        assert_eq!(atomic_audit(&engine), before);
    }

    /// Task 9 accounting seam: the engine reports its retained
    /// dependency-quarantine bytes (the exact pending payload length) and
    /// returns to zero once the dependency completes.
    #[test]
    fn pending_remote_dependency_bytes_tracks_the_quarantine_lifecycle() {
        use yrs::{diff_updates_v1, encode_state_vector_from_update_v1};

        let mut source = transaction_engine();
        let base = source.encoded_state().unwrap();
        source
            .apply_command(220, TypedCommand::InsertText { text: "q".into() })
            .unwrap();
        let after = source.encoded_state().unwrap();
        let base_sv = encode_state_vector_from_update_v1(&base).unwrap();
        let delta = diff_updates_v1(&after, &base_sv).unwrap();

        let mut target = transaction_engine();
        assert_eq!(target.pending_remote_dependency_bytes(), 0);

        // transaction_engine() starts from a different lineage than
        // `source`, so the delta's dependencies are missing and quarantine.
        assert!(!target.apply_remote_update_v1(221, &delta).unwrap().changed);
        assert_eq!(target.pending_remote_dependency_bytes(), delta.len());

        assert!(target.apply_remote_update_v1(222, &base).unwrap().changed);
        assert_eq!(target.pending_remote_dependency_bytes(), 0);
        assert_eq!(target.document().unwrap().root().text_content(), "q");
    }

    #[test]
    fn state_only_boundary_reservation_failure_is_fully_atomic() {
        use crate::yrs_engine::history::set_boundary_reservation_failure_for_test;

        let mut engine = transaction_engine();
        let before = atomic_audit(&engine);
        set_boundary_reservation_failure_for_test(true);

        let error = engine
            .apply_command(
                90,
                crate::yrs_engine::TypedCommand::ToggleMark {
                    mark_type: "bold".into(),
                },
            )
            .unwrap_err();

        set_boundary_reservation_failure_for_test(false);
        assert_eq!(error.code, "OPERATION_RESOURCE_EXHAUSTED");
        assert_eq!(atomic_audit(&engine), before);
    }

    /// Task 16B: the quarantined remote-update reservation is a demonstrated
    /// fallible allocation seam and keeps OPERATION_RESOURCE_EXHAUSTED.
    #[test]
    fn quarantined_remote_update_reservation_failure_keeps_resource_exhausted() {
        use yrs::{diff_updates_v1, encode_state_vector_from_update_v1};

        let mut source = transaction_engine();
        let base = source.encoded_state().unwrap();
        source
            .apply_command(220, TypedCommand::InsertText { text: "q".into() })
            .unwrap();
        let after = source.encoded_state().unwrap();
        let base_sv = encode_state_vector_from_update_v1(&base).unwrap();
        let delta = diff_updates_v1(&after, &base_sv).unwrap();

        let mut target = transaction_engine();
        let before = atomic_audit(&target);
        super::set_quarantined_update_reservation_failure_for_test(true);
        let error = target.apply_remote_update_v1(221, &delta).unwrap_err();
        super::set_quarantined_update_reservation_failure_for_test(false);
        assert_eq!(error.code, "OPERATION_RESOURCE_EXHAUSTED");
        assert_eq!(error.details, Some(json!({ "field": "remoteUpdate" })));
        assert_eq!(atomic_audit(&target), before);
        // Recovery: the identical update quarantines once allocation recovers.
        assert!(!target.apply_remote_update_v1(221, &delta).unwrap().changed);
    }

    /// Task 16B: the outbound staging-copy allocation seam keeps
    /// OPERATION_RESOURCE_EXHAUSTED.
    #[test]
    fn outbound_staging_copy_allocation_failure_keeps_resource_exhausted() {
        let limits = crate::session::CollaborationLimits::default();
        let mut outbox = crate::collaboration_runtime::CollaborationOutbox::from_limits(&limits);
        let mut sink = OutboundUpdateSink::attached(&mut outbox);
        super::set_outbound_staging_copy_failure_for_test(true);
        let error = sink.reserve_and_stage(41, 4, &[1, 2, 3]).unwrap_err();
        super::set_outbound_staging_copy_failure_for_test(false);
        assert_eq!(error.code, "OPERATION_RESOURCE_EXHAUSTED");
        assert_eq!(
            error.details,
            Some(json!({ "field": "pendingOutboxUpdateBytes" }))
        );
        sink.reserve_and_stage(41, 4, &[1, 2, 3]).unwrap();
    }

    /// Task 6 fix round 1: exact/one-over coverage of the shared
    /// `maxEncodedStateBytes` gate used by the remote pipeline and the sealed
    /// state-vector/diff encoders. The state-vector *output* branch is
    /// unreachable through any consistent engine (the full encoded state is
    /// strictly larger than its state vector and is bounded by the same
    /// ceiling on every admission path), so the gate is proven here at the
    /// boundary instead.
    #[test]
    fn max_encoded_state_gate_admits_exact_and_rejects_one_over() {
        assert!(super::admit_max_encoded_state_len(90_001, 64, 64).is_ok());
        assert!(super::admit_max_encoded_state_len(90_002, 0, 0).is_ok());

        let error = super::admit_max_encoded_state_len(90_003, 65, 64).unwrap_err();
        assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
        assert_eq!(error.request_id, 90_003);
        assert_eq!(error.limit, Some(64));
        assert_eq!(error.actual, Some(65));
        assert_eq!(
            error.details.as_ref().unwrap()["field"],
            "maxEncodedStateBytes"
        );
    }

    /// Task 6 same-doc binding proof: the codec's sole `Awareness` wraps the
    /// live authoritative `Doc` handle (documents edits are visible through
    /// it, the client identity matches), and the binding follows every store
    /// swap (undo/redo candidate installation and import).
    #[test]
    fn awareness_codec_owns_an_awareness_bound_to_the_live_doc() {
        use yrs::GetString;

        fn bound_fragment_text(engine: &YrsDocumentEngine) -> String {
            let codec = engine.awareness.as_ref().expect("codec stays bound");
            let doc = codec.doc_for_test();
            assert!(
                Doc::ptr_eq(doc, &engine.doc),
                "awareness must wrap the live authoritative doc handle"
            );
            assert_eq!(doc.client_id().get(), engine.client_id());
            let txn = doc.transact();
            txn.get_xml_fragment("prosemirror")
                .expect("live doc retains the document fragment")
                .get_string(&txn)
        }

        let mut engine =
            transaction_engine_with_editing_limits(crate::yrs_engine::EditingLimits::default());
        engine.awareness();

        engine
            .apply_command(
                1,
                TypedCommand::InsertText {
                    text: "bound".into(),
                },
            )
            .unwrap()
            .expect("insert applies");
        assert!(bound_fragment_text(&engine).contains("bound"));

        engine.undo(2).unwrap().expect("undo applies");
        assert!(!bound_fragment_text(&engine).contains("bound"));

        engine
            .import_json(
                &json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"imported"}]}]})
                    .to_string(),
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        assert!(bound_fragment_text(&engine).contains("imported"));
    }
}
