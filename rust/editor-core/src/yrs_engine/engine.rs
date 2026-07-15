use serde_json::json;
use std::collections::HashSet;
use yrs::updates::decoder::Decode;
use yrs::Update;
use yrs::{Doc, OffsetKind, Options, ReadTxn, StateVector, Transact, WriteTxn};

use crate::boundary::{BoundedInput, InputKind, ResourceLimits};
use crate::model::Document;
use crate::schema::{schema_fingerprint, NodeRole, Schema};
use crate::serialize::{
    from_html_with_limits, from_prosemirror_json_with_limits, rehydrate_reserved_html_opaque,
    to_html, to_prosemirror_json, FromHtmlOptions, JsonParseError, ParseError, UnknownTypeMode,
};
use crate::transform::{validate_canonical_marks, DocumentValidator};

use super::compiler::{compile_transaction_with_yrs, CompilationContext, CompiledTransaction};
use super::mutation::{execute_mutation_plan, preflight_mutation_plan};
use super::update_preflight::preflight_update_v1;
use super::{
    DocumentScope, DocumentSnapshot, EditingLimits, TransactionOrigin, YrsDocumentCodec,
    YrsEngineError, YrsEngineResult, SNAPSHOT_FORMAT_VERSION,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitializationMode {
    LocalEmpty,
    AwaitRemote,
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

enum EngineDocumentState {
    AwaitingRemote,
    Ready {
        document: Document,
        canonical_json: serde_json::Value,
    },
}

struct CandidateDocument {
    doc: Doc,
    state: EngineDocumentState,
    durable_client_ids: HashSet<u64>,
}

struct ValidatedImportDocument {
    document: Document,
    canonical_json: serde_json::Value,
}

impl ValidatedImportDocument {
    fn new(
        document: Document,
        schema: &Schema,
        resource_limits: &ResourceLimits,
    ) -> YrsEngineResult<Self> {
        if contains_reserved_public_json_forge(document.root()) {
            return Err(candidate_invariant_parse_error(
                "public JSON cannot construct reserved opaque HTML metadata",
                "candidate codec round-trip changed the document",
            ));
        }
        validate_yrs_mark_representation(&document, schema)?;
        validate_import_document(&document, schema, resource_limits)?;
        let canonical_json = to_prosemirror_json(&document, schema);
        Ok(Self {
            document,
            canonical_json,
        })
    }
}

fn contains_reserved_public_json_forge(node: &crate::model::Node) -> bool {
    if node.node_type() == "__opaque_json"
        && node
            .attrs()
            .get("original_type")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|node_type| matches!(node_type, "__opaque" | "__opaque_json" | "__skip"))
    {
        return true;
    }
    node.content()
        .is_some_and(|content| content.iter().any(contains_reserved_public_json_forge))
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
    state: EngineDocumentState,
    revision: u64,
    state_revision: u64,
    yrs_state_epoch: u64,
    last_committed_origin: Option<TransactionOrigin>,
    durable_client_ids: HashSet<u64>,
}

impl YrsDocumentEngine {
    pub fn new(config: YrsEngineConfig) -> YrsEngineResult<Self> {
        let YrsEngineConfig {
            schema,
            fragment_name,
            initialization_mode,
            resource_limits,
            editing_limits,
            max_length,
            scope,
        } = config;
        validate_config_metadata(&fragment_name, scope.as_ref(), &resource_limits)?;
        let schema_fingerprint = schema_fingerprint(&schema);
        let candidate = match initialization_mode {
            InitializationMode::LocalEmpty => {
                build_local_empty_candidate(&schema, &fragment_name, &resource_limits)?
            }
            InitializationMode::AwaitRemote => {
                build_await_remote_candidate(&fragment_name, &resource_limits)?
            }
        };

        Ok(Self {
            doc: candidate.doc,
            fragment_name,
            schema,
            resource_limits,
            editing_limits,
            max_length,
            scope,
            schema_fingerprint,
            state: candidate.state,
            revision: 0,
            state_revision: 0,
            yrs_state_epoch: 0,
            last_committed_origin: None,
            durable_client_ids: candidate.durable_client_ids,
        })
    }

    pub fn is_ready(&self) -> bool {
        matches!(self.state, EngineDocumentState::Ready { .. })
    }

    pub fn document(&self) -> Option<&Document> {
        match &self.state {
            EngineDocumentState::AwaitingRemote => None,
            EngineDocumentState::Ready { document, .. } => Some(document),
        }
    }

    pub fn document_json(&self) -> Option<serde_json::Value> {
        match &self.state {
            EngineDocumentState::AwaitingRemote => None,
            EngineDocumentState::Ready { canonical_json, .. } => Some(canonical_json.clone()),
        }
    }

    pub fn document_html(&self) -> Option<String> {
        self.document()
            .map(|document| to_html(document, &self.schema))
    }

    pub fn encoded_state(&self) -> YrsEngineResult<Vec<u8>> {
        encode_state_bounded(&self.doc, &self.resource_limits)
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn state_revision(&self) -> u64 {
        self.state_revision
    }

    pub fn client_id(&self) -> u64 {
        self.doc.client_id().get()
    }

    pub fn fragment_name(&self) -> &str {
        &self.fragment_name
    }

    pub fn schema_fingerprint(&self) -> &str {
        &self.schema_fingerprint
    }

    pub fn scope(&self) -> Option<&DocumentScope> {
        self.scope.as_ref()
    }

    pub fn last_committed_origin(&self) -> Option<TransactionOrigin> {
        self.last_committed_origin
    }

    pub fn resource_limits(&self) -> &ResourceLimits {
        &self.resource_limits
    }

    pub fn editing_limits(&self) -> &EditingLimits {
        &self.editing_limits
    }

    pub fn max_length(&self) -> Option<u32> {
        self.max_length
    }

    #[allow(dead_code)] // Task 7 exposes the internal compiler through atomic application.
    pub(crate) fn compile_typed_transaction(
        &self,
        transaction: super::TypedTransaction,
    ) -> super::OperationResult<CompiledTransaction> {
        let document = self
            .document()
            .ok_or_else(|| super::OperationError::engine_not_ready(transaction.request_id))?;
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
        let mut compiled = compile_transaction_with_yrs(
            CompilationContext {
                document,
                selection: None,
                schema: &self.schema,
                resource_limits: &self.resource_limits,
                editing_limits: &self.editing_limits,
                document_revision: self.revision,
                max_length: self.max_length,
            },
            transaction,
            &txn,
            &fragment,
        )?;
        compiled.yrs_state_epoch = self.yrs_state_epoch;
        Ok(compiled)
    }

    pub fn apply_typed_transaction(
        &mut self,
        transaction: super::TypedTransaction,
    ) -> super::OperationResult<super::TransactionCommit> {
        let compiled = self.compile_typed_transaction(transaction)?;
        self.apply_compiled_transaction(compiled)
    }

    fn apply_compiled_transaction(
        &mut self,
        mut compiled: CompiledTransaction,
    ) -> super::OperationResult<super::TransactionCommit> {
        // A compiled plan owns Yrs handles after its original read transaction
        // closes. Reject a stale plan in O(1) before no-op classification or
        // any state-vector/snapshot traversal.
        if compiled.yrs_state_epoch != self.yrs_state_epoch {
            return Err(super::OperationError::engine_invariant_failed(
                compiled.request_id,
                None,
                "compiled Yrs transaction is stale",
            ));
        }
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
        if preview_is_unchanged {
            return Ok(super::TransactionCommit {
                request_id: compiled.request_id,
                changed: false,
                document_revision: self.revision,
                state_revision: self.state_revision,
                origin: compiled.origin,
            });
        }

        #[cfg(test)]
        super::compiler::check_atomic_failpoint(
            compiled.request_id,
            super::compiler::AtomicFailpoint::CanonicalOutputAdmission,
        )?;
        let canonical_json = compiled.canonical_json.take().ok_or_else(|| {
            super::OperationError::engine_invariant_failed(
                compiled.request_id,
                None,
                "changed transaction has no admitted canonical JSON",
            )
        })?;

        // Revalidate sealed signatures against one final stable read view.
        let current_encoded_bytes = {
            let txn = self.doc.transact();
            #[cfg(test)]
            super::compiler::check_atomic_failpoint(
                compiled.request_id,
                super::compiler::AtomicFailpoint::FinalPreflight,
            )?;
            preflight_mutation_plan(compiled.request_id, &compiled.mutation_plan, &txn)?;
            #[cfg(test)]
            super::compiler::check_atomic_failpoint(
                compiled.request_id,
                super::compiler::AtomicFailpoint::EncodedAdmission,
            )?;
            if txn.state_vector().is_empty() {
                0
            } else {
                txn.encode_state_as_update_v1(&StateVector::default()).len()
            }
        };
        let admitted_encoded_bytes = current_encoded_bytes
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
        #[cfg(test)]
        super::compiler::check_atomic_failpoint(
            compiled.request_id,
            super::compiler::AtomicFailpoint::DurableMetadataAdmission,
        )?;
        let mut next_durable_client_ids = self.durable_client_ids.clone();
        if compiled.authored_clock_units > 0 {
            next_durable_client_ids.insert(self.client_id());
        }

        let CompiledTransaction {
            request_id,
            origin,
            preview,
            mutation_plan,
            ..
        } = compiled;
        {
            let mut txn = self.doc.transact_mut_with(origin.as_yrs_origin());
            execute_mutation_plan(mutation_plan, &mut txn);
        }

        self.state = EngineDocumentState::Ready {
            document: preview,
            canonical_json,
        };
        self.durable_client_ids = next_durable_client_ids;
        self.revision = next_document_revision;
        self.state_revision = next_state_revision;
        self.yrs_state_epoch = next_yrs_state_epoch;
        self.last_committed_origin = Some(origin);
        Ok(super::TransactionCommit {
            request_id,
            changed: true,
            document_revision: self.revision,
            state_revision: self.state_revision,
            origin,
        })
    }

    pub fn export_snapshot(&self) -> YrsEngineResult<DocumentSnapshot> {
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
        self.validate_snapshot_manifest(snapshot)?;

        let current_state = encode_state_bounded(&self.doc, &self.resource_limits)?;
        if self.is_ready() && current_state == snapshot.encoded_state {
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

        let (next_revision, next_state_revision, next_yrs_state_epoch) =
            self.next_durable_revisions()?;
        self.doc = candidate.doc;
        self.state = candidate.state;
        self.durable_client_ids = candidate.durable_client_ids;
        self.revision = next_revision;
        self.state_revision = next_state_revision;
        self.yrs_state_epoch = next_yrs_state_epoch;
        self.last_committed_origin = Some(TransactionOrigin::SnapshotRestore);
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
        .map_err(|error| snapshot_derived_error(error, "encodedState"))?;
        let derived_document = rehydrate_reserved_html_opaque(&derived_document);
        validate_import_document(&derived_document, &self.schema, &self.resource_limits)
            .map_err(|error| snapshot_derived_error(error, "encodedState"))?;
        encode_candidate_state_bounded(&candidate_doc, &self.resource_limits)
            .map_err(|error| snapshot_derived_error(error, "encodedState"))?;
        let canonical_json = to_prosemirror_json(&derived_document, &self.schema);
        Ok(CandidateDocument {
            doc: candidate_doc,
            state: EngineDocumentState::Ready {
                document: derived_document,
                canonical_json,
            },
            durable_client_ids,
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
        let value = serde_json::from_str(input.as_str())
            .map_err(|error| YrsEngineError::parse("DOCUMENT_INVALID", error))?;
        if let EngineDocumentState::Ready { canonical_json, .. } = &self.state {
            if canonical_json == &value {
                return Ok(EngineCommit {
                    changed: false,
                    revision: self.revision,
                });
            }
        }
        let document = from_prosemirror_json_with_limits(
            &value,
            &self.schema,
            UnknownTypeMode::Preserve,
            &self.resource_limits,
        )
        .map_err(map_json_import_error)?;
        let source = ValidatedImportDocument::new(document, &self.schema, &self.resource_limits)?;

        let candidate = self.build_candidate_from_document(source, origin)?;
        self.commit_candidate(candidate, origin)
    }

    pub fn import_html(
        &mut self,
        input: &str,
        options: &FromHtmlOptions,
        origin: TransactionOrigin,
    ) -> YrsEngineResult<EngineCommit> {
        let input = BoundedInput::new(input, InputKind::Html, &self.resource_limits)?;
        let document =
            from_html_with_limits(input.as_str(), &self.schema, options, &self.resource_limits)
                .map_err(map_html_import_error)?;
        let source = ValidatedImportDocument::new(document, &self.schema, &self.resource_limits)?;

        let candidate = self.build_candidate_from_document(source, origin)?;
        self.commit_candidate(candidate, origin)
    }

    fn build_candidate_from_document(
        &self,
        source: ValidatedImportDocument,
        origin: TransactionOrigin,
    ) -> YrsEngineResult<CandidateDocument> {
        let ValidatedImportDocument {
            document: source_document,
            canonical_json,
        } = source;
        let empty_json = json!({
            "type": self.schema.doc_node_type(),
            "content": [],
        });
        let doc = fresh_utf16_doc_excluding(&self.durable_client_ids, self.client_id());
        let codec = YrsDocumentCodec::new(&self.schema, &self.resource_limits);
        {
            let mut txn = doc.transact_mut_with(origin.as_yrs_origin());
            let fragment = txn.get_or_insert_xml_fragment(self.fragment_name.as_str());
            codec.apply_json(&fragment, &mut txn, &empty_json, &canonical_json)?;
        }

        let derived_json = {
            let txn = doc.transact();
            let fragment = txn
                .get_xml_fragment(self.fragment_name.as_str())
                .ok_or_else(candidate_invariant_error)?;
            codec.read_json_from_validated_source(&fragment, &txn)?
        };
        let derived_document = from_prosemirror_json_with_limits(
            &derived_json,
            &self.schema,
            UnknownTypeMode::Preserve,
            &self.resource_limits,
        )
        .map_err(|error| candidate_invariant_parse_error(error, "derived document is invalid"))?;
        let derived_document = rehydrate_reserved_html_opaque(&derived_document);
        DocumentValidator::validate(&derived_document, &self.schema, &self.resource_limits)
            .map_err(|error| {
                candidate_invariant_parse_error(error, "derived document is invalid")
            })?;
        if derived_document != source_document {
            return Err(candidate_invariant_parse_error(
                "derived document does not match the validated import",
                "candidate codec round-trip changed the document",
            ));
        }
        encode_candidate_state_bounded(&doc, &self.resource_limits)?;

        let durable_client_ids = HashSet::from([doc.client_id().get()]);
        Ok(CandidateDocument {
            doc,
            state: EngineDocumentState::Ready {
                document: derived_document,
                canonical_json,
            },
            durable_client_ids,
        })
    }

    fn commit_candidate(
        &mut self,
        candidate: CandidateDocument,
        origin: TransactionOrigin,
    ) -> YrsEngineResult<EngineCommit> {
        let candidate_document = match &candidate.state {
            EngineDocumentState::Ready { document, .. } => document,
            EngineDocumentState::AwaitingRemote => {
                unreachable!("imports always build ready candidates")
            }
        };
        let unchanged = match &self.state {
            EngineDocumentState::Ready { document, .. } => document == candidate_document,
            EngineDocumentState::AwaitingRemote => false,
        };
        if unchanged {
            return Ok(EngineCommit {
                changed: false,
                revision: self.revision,
            });
        }

        let (next_revision, next_state_revision, next_yrs_state_epoch) =
            self.next_durable_revisions()?;
        self.doc = candidate.doc;
        self.state = candidate.state;
        self.durable_client_ids = candidate.durable_client_ids;
        self.revision = next_revision;
        self.state_revision = next_state_revision;
        self.yrs_state_epoch = next_yrs_state_epoch;
        self.last_committed_origin = Some(origin);
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

fn snapshot_error(
    code: &'static str,
    message: impl Into<String>,
    field: &'static str,
) -> YrsEngineError {
    YrsEngineError::new(code, message).with_details(json!({ "field": field }))
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

fn validate_yrs_mark_representation(document: &Document, schema: &Schema) -> YrsEngineResult<()> {
    validate_canonical_marks(document, schema).map_err(|error| YrsEngineError {
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
    DocumentValidator::validate(document, schema, resource_limits)
        .map(|_| ())
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
    let canonical_json = to_prosemirror_json(&document, schema);
    encode_state_bounded(&doc, resource_limits)?;

    let durable_client_ids = HashSet::from([doc.client_id().get()]);
    Ok(CandidateDocument {
        doc,
        state: EngineDocumentState::Ready {
            document,
            canonical_json,
        },
        durable_client_ids,
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
    })
}

fn utf16_doc() -> Doc {
    let options = Options {
        offset_kind: OffsetKind::Utf16,
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

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use crate::boundary::ResourceLimits;
    use crate::model::Mark;
    use crate::schema::presets::tiptap_schema;
    use crate::serialize::{from_prosemirror_json, UnknownTypeMode};
    use serde_json::json;
    use yrs::OffsetKind;

    use yrs::{updates::decoder::Decode, Update};
    use yrs::{ClientID, Doc, Options};

    use crate::yrs_engine::{
        Affinity, EditorOffsetKind, HistoryPolicy, RevisionedPosition, RevisionedRange,
        SelectionIntent, TransactionOrigin, TypedOperation, TypedTransaction,
    };

    use super::{
        fresh_utf16_doc_excluding_with, utf16_doc, ValidatedImportDocument, YrsDocumentEngine,
        YrsEngineConfig,
    };

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
        }
    }

    fn transaction_engine() -> YrsDocumentEngine {
        YrsDocumentEngine::new(YrsEngineConfig {
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
        .unwrap()
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

        let validated = ValidatedImportDocument::new(parsed, &schema, &limits).unwrap();

        assert_eq!(
            validated.canonical_json,
            json!({
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
            validated.canonical_json,
            crate::serialize::to_prosemirror_json(&validated.document, &schema)
        );
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
                "documentRevision" => engine.revision = u64::MAX,
                "stateRevision" => engine.state_revision = u64::MAX,
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

            let error = engine.apply_compiled_transaction(compiled).unwrap_err();

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

        assert!(target.durable_client_ids.contains(&local_client));
        let durable_clients = Update::decode_v1(&target.encoded_state().unwrap())
            .unwrap()
            .state_vector();
        assert!(durable_clients.get(&ClientID::new(local_client)) > 0);
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
        }
    }
}
