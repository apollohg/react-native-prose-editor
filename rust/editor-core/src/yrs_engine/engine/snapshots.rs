use super::candidate::{
    admit_candidate_derived_output, build_derived_state_for_candidate, CandidateDocument,
    EngineDocumentState,
};
use super::candidate_cache::{
    encode_candidate_state_bounded, encode_state_bounded, fresh_utf16_doc_excluding,
};
use super::imports::{map_json_import_error, validate_import_document};
use super::{EngineCommit, YrsDocumentEngine};
use crate::boundary::ResourceLimits;
use crate::serialize::{
    from_prosemirror_json_with_limits, rehydrate_reserved_html_opaque, UnknownTypeMode,
};
use crate::yrs_engine;
use crate::yrs_engine::update_preflight::preflight_update_v1;
use crate::yrs_engine::{
    DocumentScope, DocumentSnapshot, TransactionOrigin, YrsDocumentCodec, YrsEngineError,
    YrsEngineResult, SNAPSHOT_FORMAT_VERSION,
};
use serde_json::json;
use yrs::updates::decoder::Decode;
use yrs::{ReadTxn, Transact, Update};

impl YrsDocumentEngine {
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
        self.document_origin = yrs_engine::DocumentOrigin::Restore;
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
