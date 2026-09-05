use super::candidate_cache::{
    encode_candidate_state_bounded, encode_state_bounded, fresh_utf16_doc_excluding,
    prepare_import_candidate_cache, utf16_doc, ImportEncodedStateReceipt,
};
use super::imports::{RootBoundValidationReport, ValidatedImportDocument};
use super::{EngineCommit, YrsDocumentEngine};
use crate::boundary::ResourceLimits;
use crate::model::Document;
use crate::schema::Schema;
use crate::serialize::{from_prosemirror_json_with_limits, to_prosemirror_json, UnknownTypeMode};
use crate::transform::DocumentValidator;
use crate::yrs_engine;
use crate::yrs_engine::canonical::{CanonicalArtifact, CanonicalSchemaContext};
use crate::yrs_engine::derived_state::{
    DerivedStateCache, ValidatedCandidateContext, ValidatedDocumentEvidence,
};
use crate::yrs_engine::{
    EditingLimits, TransactionOrigin, YrsDocumentCodec, YrsEngineError, YrsEngineResult,
};
use serde_json::json;
use std::collections::HashSet;
use yrs::{Doc, ReadTxn, Transact, WriteTxn};

pub(super) enum EngineDocumentState {
    AwaitingRemote,
    Ready {
        document: Document,
        canonical_artifact: CanonicalArtifact,
    },
}

pub(super) struct CandidateDocument {
    pub(super) doc: Doc,
    pub(super) state: EngineDocumentState,
    pub(super) durable_client_ids: HashSet<u64>,
    pub(super) validated_import: Option<RootBoundValidationReport>,
    pub(super) import_acceleration_eligible: bool,
    pub(super) import_encoded_state_receipt: Option<ImportEncodedStateReceipt>,
}

impl YrsDocumentEngine {
    pub(super) fn build_candidate_from_document(
        &self,
        source: ValidatedImportDocument,
        origin: TransactionOrigin,
    ) -> YrsEngineResult<CandidateDocument> {
        let doc = fresh_utf16_doc_excluding(&self.durable_client_ids, self.client_id());
        self.build_candidate_from_document_in_doc(source, origin, doc)
    }

    pub(super) fn build_candidate_from_document_in_doc(
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

    pub(super) fn commit_candidate(
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
                    yrs_engine::mutation::MutationLookupSeed::from_import_materialization(
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
                    yrs_engine::mutation::MutationLookupSeed::build(
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
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build_derived_state_for_candidate(
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

pub(super) fn admit_candidate_derived_output(
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

pub(super) fn admit_canonical_output(
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

fn candidate_invariant_error() -> YrsEngineError {
    candidate_invariant_parse_error(
        "candidate Yrs fragment is missing",
        "candidate Yrs fragment is missing",
    )
}

pub(super) fn candidate_invariant_parse_error(
    error: impl std::fmt::Display,
    message: &'static str,
) -> YrsEngineError {
    YrsEngineError::new("CODEC_INVARIANT_FAILED", format!("{message}: {error}"))
        .with_details(json!({ "phase": "candidateDerivation" }))
}

pub(super) fn build_local_empty_candidate(
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

pub(super) fn build_await_remote_candidate(
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
