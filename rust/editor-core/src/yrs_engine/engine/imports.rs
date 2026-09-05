use super::candidate::{admit_canonical_output, candidate_invariant_parse_error};
use super::outbound::OutboundUpdateSink;
use super::{EngineCommit, YrsDocumentEngine};
use crate::boundary::{
    document_json_container_depth_limit, parse_json_value_stack_safe,
    with_document_stack_for_json_container_depth, BoundedInput, InputKind, ResourceLimits,
};
use crate::model::Document;
use crate::schema::{NodeRole, Schema};
use crate::selection::Selection;
use crate::serialize::{
    from_html_with_limits, from_prosemirror_json_with_limits, FromHtmlOptions, JsonParseError,
    ParseError, UnknownTypeMode,
};
use crate::transform::{
    canonicalize_yrs_document_with_evidence, validate_importable_marks_with_evidence,
    CanonicalMarksEvidence, DocumentValidationReport, DocumentValidator,
};
use crate::yrs_engine;
use crate::yrs_engine::canonical::{CanonicalArtifact, CanonicalSchemaContext};
use crate::yrs_engine::{EditingLimits, TransactionOrigin, YrsEngineError, YrsEngineResult};

#[derive(Clone)]
pub(super) struct RootBoundValidationReport {
    pub(super) source_root: crate::model::Node,
    pub(super) report: DocumentValidationReport,
}

pub(super) struct ValidatedImportDocument {
    pub(super) document: Document,
    pub(super) canonical_artifact: CanonicalArtifact,
    pub(super) validation: RootBoundValidationReport,
    pub(super) carry_import_encoded_state_receipt: bool,
}

impl ValidatedImportDocument {
    pub(super) fn new(
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

impl YrsDocumentEngine {
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
        yrs_engine::observability::record_import_model_parse();
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
        yrs_engine::observability::record_import_model_parse();
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
        history: yrs_engine::ReplacementHistory,
    ) -> Result<yrs_engine::TransactionCommit, yrs_engine::RootReplacementError> {
        self.prepare_root_replacement_json_with_outbox(request_id, input, history, None)
    }

    /// [`Self::prepare_root_replacement_json`] with an optionally attached
    /// collaboration outbox for bounded outbound update capture.
    pub(crate) fn prepare_root_replacement_json_with_outbox(
        &mut self,
        request_id: u64,
        input: &str,
        history: yrs_engine::ReplacementHistory,
        outbox: Option<&mut crate::collaboration_runtime::CollaborationOutbox>,
    ) -> Result<yrs_engine::TransactionCommit, yrs_engine::RootReplacementError> {
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
        history: yrs_engine::ReplacementHistory,
    ) -> Result<yrs_engine::TransactionCommit, yrs_engine::RootReplacementError> {
        self.prepare_root_replacement_html_with_outbox(request_id, input, options, history, None)
    }

    /// [`Self::prepare_root_replacement_html`] with an optionally attached
    /// collaboration outbox for bounded outbound update capture.
    pub(crate) fn prepare_root_replacement_html_with_outbox(
        &mut self,
        request_id: u64,
        input: &str,
        options: &FromHtmlOptions,
        history: yrs_engine::ReplacementHistory,
        outbox: Option<&mut crate::collaboration_runtime::CollaborationOutbox>,
    ) -> Result<yrs_engine::TransactionCommit, yrs_engine::RootReplacementError> {
        use yrs_engine::RootReplacementError;
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
    ) -> Result<ValidatedImportDocument, yrs_engine::RootReplacementError> {
        use yrs_engine::RootReplacementError;
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
        history: yrs_engine::ReplacementHistory,
    ) -> Result<yrs_engine::TypedTransaction, yrs_engine::RootReplacementError> {
        use yrs_engine::RootReplacementError;
        let current = self.document().ok_or_else(|| {
            RootReplacementError::Transaction(yrs_engine::OperationError::engine_not_ready(
                request_id,
            ))
        })?;
        let root_children = u32::try_from(current.root().child_count()).map_err(|_| {
            RootReplacementError::Transaction(yrs_engine::OperationError::engine_invariant_failed(
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
            yrs_engine::ReplacementHistory::UndoableBoundary => yrs_engine::HistoryPolicy::Boundary,
            yrs_engine::ReplacementHistory::ResetAndClear => yrs_engine::HistoryPolicy::Skip,
        };
        Ok(yrs_engine::TypedTransaction {
            request_id,
            base_document_revision: self.revision,
            origin: TransactionOrigin::LocalApi,
            operations: vec![yrs_engine::TypedOperation::ReplaceStructure(
                yrs_engine::StructuralReplacement::new(
                    Vec::new(),
                    0,
                    root_children,
                    content,
                    Selection::cursor(0),
                ),
            )],
            selection_intent: yrs_engine::SelectionIntent::UseOperationResult,
            history_policy,
        })
    }

    /// Lower an admitted replacement document to one sealed whole-root
    /// `ReplaceStructure` transaction and apply the requested history class.
    fn commit_root_replacement(
        &mut self,
        request_id: u64,
        source: ValidatedImportDocument,
        history: yrs_engine::ReplacementHistory,
        outbox: Option<&mut crate::collaboration_runtime::CollaborationOutbox>,
    ) -> Result<yrs_engine::TransactionCommit, yrs_engine::RootReplacementError> {
        use yrs_engine::RootReplacementError;
        let transaction = self.root_replacement_transaction(request_id, &source, history)?;
        let (commit, _) = self
            .apply_typed_transaction_with_staged_context(
                transaction,
                false,
                &mut OutboundUpdateSink::from_optional_outbox(outbox),
            )
            .map_err(RootReplacementError::Transaction)?;
        if history == yrs_engine::ReplacementHistory::ResetAndClear {
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
        history: yrs_engine::ReplacementHistory,
    ) -> Result<usize, yrs_engine::RootReplacementError> {
        let source = self.admit_root_replacement_json(input)?;
        let transaction = self.root_replacement_transaction(request_id, &source, history)?;
        self.compile_typed_transaction(transaction)
            .map(|compiled| compiled.outbound_update_upper_bound())
            .map_err(yrs_engine::RootReplacementError::Transaction)
    }
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

pub(super) fn validate_import_document(
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

pub(super) fn map_json_import_error(error: JsonParseError) -> YrsEngineError {
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
