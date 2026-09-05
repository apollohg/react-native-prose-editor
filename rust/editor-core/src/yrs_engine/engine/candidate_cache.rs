#[cfg(test)]
use super::test_hooks::{
    CANDIDATE_BOUNDED_STATE_ENCODINGS, IMPORT_CANDIDATE_STATE_ENCODINGS,
    IMPORT_RECEIPT_SHA256_MATCHES, IMPORT_RECEIPT_SHA256_MINTS, IMPORT_RECEIPT_STATE_DECODINGS,
};
use crate::boundary::ResourceLimits;
use crate::model::Document;
use crate::schema::Schema;
use crate::yrs_engine;
use crate::yrs_engine::canonical::CanonicalArtifact;
use crate::yrs_engine::mutation::YrsMutationPlan;
use crate::yrs_engine::{EditingLimits, YrsEngineError, YrsEngineResult};
use serde_json::json;
use sha2::Digest;
use std::collections::HashSet;
use std::sync::Arc;
use yrs::branch::{Branch, BranchID};
use yrs::types::xml::XmlFragmentRef;
use yrs::updates::decoder::Decode;
use yrs::{ClientID, Doc, OffsetKind, Options, ReadTxn, StateVector, Transact, Update, Uuid};

/// A one-owner capability proving that these exact standard update-v1 bytes
/// were produced from the validated import candidate after its codec
/// round-trip and encoded-state admission completed.
pub(super) struct ImportEncodedStateReceipt {
    pub(super) encoded_state: Vec<u8>,
    pub(super) encoded_state_sha256: [u8; 32],
    pub(super) state_vector: StateVector,
    pub(super) fragment_id: BranchID,
    pub(super) client_id: ClientID,
    pub(super) guid: Uuid,
    pub(super) offset_kind: OffsetKind,
    pub(super) skip_gc: bool,
    pub(super) delete_set_is_empty: bool,
    pub(super) lookup_materialization: Option<ImportLookupMaterializationReceipt>,
    pub(super) lookup_state_verified: bool,
}

pub(super) struct ImportLookupMaterializationReceipt {
    pub(super) materialization: yrs_engine::mutation::ImportLookupMaterialization,
    pub(super) source_document: Document,
    pub(super) canonical_artifact: CanonicalArtifact,
    pub(super) resource_limits: ResourceLimits,
    pub(super) editing_limits: EditingLimits,
    pub(super) max_length: Option<u32>,
    pub(super) schema_token: usize,
    pub(super) store_token: usize,
}

pub(super) struct FinalizedImportLookupMaterialization {
    pub(super) materialization: yrs_engine::mutation::ImportLookupMaterialization,
    pub(super) source_document: Document,
    pub(super) canonical_artifact: CanonicalArtifact,
    pub(super) resource_limits: ResourceLimits,
    pub(super) editing_limits: EditingLimits,
    pub(super) max_length: Option<u32>,
    pub(super) document_revision: u64,
    pub(super) yrs_state_epoch: u64,
}

impl ImportEncodedStateReceipt {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn mint(
        source: &Doc,
        fragment_name: &str,
        encoded_state: Vec<u8>,
        delete_set_is_empty: bool,
        lookup_materialization: Option<yrs_engine::mutation::ImportLookupMaterialization>,
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
    pub(super) fn take_matching_lookup_materialization(
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

pub(super) struct PreparedCandidateCache {
    pub(super) doc: Doc,
    pub(super) state_vector: StateVector,
    pub(super) staged_lookup_seed: Option<Arc<yrs_engine::mutation::MutationLookupSeed>>,
    pub(super) document_revision: u64,
    pub(super) yrs_state_epoch: u64,
    pub(super) encoded_state_seal: Option<EncodedStateSeal>,
}

pub(super) struct EncodedStateSeal {
    pub(super) encoded_state: Vec<u8>,
    pub(super) fragment_id: BranchID,
    pub(super) client_id: ClientID,
    pub(super) guid: Uuid,
    pub(super) offset_kind: OffsetKind,
    pub(super) skip_gc: bool,
    pub(super) document_revision: u64,
    pub(super) yrs_state_epoch: u64,
}

impl PreparedCandidateCache {
    pub(super) fn take_matching_encoded_state(
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

    pub(super) fn into_matching_doc(
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
    pub(super) fn store_token(&self) -> usize {
        let txn = self.doc.transact();
        txn.store() as *const _ as usize
    }
}

pub(super) fn retained_import_state_charge(
    encoded_len: usize,
    encoded_capacity: usize,
) -> Option<usize> {
    encoded_len.checked_mul(2)?.checked_add(encoded_capacity)
}

pub(super) fn seal_candidate_state_vector(
    request_id: u64,
    base: &StateVector,
    actual: StateVector,
    local_client: ClientID,
    admitted_authored_clock_bound: u32,
) -> yrs_engine::OperationResult<StateVector> {
    let base_local_clock = base.get(&local_client);
    let actual_local_clock = actual.get(&local_client);
    let Some(actual_local_delta) = actual_local_clock.checked_sub(base_local_clock) else {
        return Err(yrs_engine::OperationError::engine_invariant_failed(
            request_id,
            None,
            "prepared commit candidate regressed its local authored clock",
        ));
    };
    if actual_local_delta > admitted_authored_clock_bound {
        return Err(yrs_engine::OperationError::engine_invariant_failed(
            request_id,
            None,
            "prepared commit candidate exceeded its admitted authored clock bound",
        ));
    }
    let mut expected = base.clone();
    expected.inc_by(local_client, actual_local_delta);
    if actual != expected {
        return Err(yrs_engine::OperationError::engine_invariant_failed(
            request_id,
            None,
            "prepared commit candidate changed a nonlocal authored clock",
        ));
    }
    Ok(actual)
}

pub(super) fn utf16_doc() -> Doc {
    let options = Options {
        offset_kind: OffsetKind::Utf16,
        // Yrs history StackItems refer to deleted structs. Keep them available
        // in both live and candidate stores for the lifetime of an epoch.
        skip_gc: true,
        ..Options::default()
    };
    Doc::with_options(options)
}

pub(super) fn fresh_utf16_doc_excluding(
    durable_client_ids: &HashSet<u64>,
    previous_client_id: u64,
) -> Doc {
    fresh_utf16_doc_excluding_with(durable_client_ids, previous_client_id, utf16_doc)
}

pub(super) fn fresh_utf16_doc_excluding_with(
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

pub(super) fn encode_state_bounded(
    doc: &Doc,
    resource_limits: &ResourceLimits,
) -> YrsEngineResult<Vec<u8>> {
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

pub(super) fn encode_candidate_state_bounded(
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

pub(super) fn equivalent_private_candidate_doc(source: &Doc) -> Doc {
    Doc::with_options(Options {
        client_id: source.client_id(),
        guid: source.guid(),
        offset_kind: source.offset_kind(),
        skip_gc: source.skip_gc(),
        ..Options::default()
    })
}

pub(super) fn prepare_import_candidate_cache(
    source: &Doc,
    fragment_name: &str,
    resource_limits: &ResourceLimits,
    import_encoded_state_receipt: Option<ImportEncodedStateReceipt>,
    staged_lookup_seed: Option<Arc<yrs_engine::mutation::MutationLookupSeed>>,
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
