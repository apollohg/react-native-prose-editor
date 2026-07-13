use serde_json::json;
use yrs::{Doc, OffsetKind, Options, ReadTxn, StateVector, Transact, WriteTxn};

use crate::boundary::{BoundedInput, InputKind, ResourceLimits};
use crate::model::{Document, Fragment, Node};
use crate::schema::{schema_fingerprint, NodeRole, Schema};
use crate::serialize::{
    from_html_with_limits, from_prosemirror_json_with_limits, to_html, to_prosemirror_json,
    FromHtmlOptions, JsonParseError, ParseError, UnknownTypeMode,
};
use crate::transform::DocumentValidator;

use super::{DocumentScope, TransactionOrigin, YrsDocumentCodec, YrsEngineError, YrsEngineResult};

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
}

pub struct YrsDocumentEngine {
    doc: Doc,
    fragment_name: String,
    schema: Schema,
    resource_limits: ResourceLimits,
    scope: Option<DocumentScope>,
    schema_fingerprint: String,
    state: EngineDocumentState,
    revision: u64,
    last_committed_origin: Option<TransactionOrigin>,
}

impl YrsDocumentEngine {
    pub fn new(config: YrsEngineConfig) -> YrsEngineResult<Self> {
        let YrsEngineConfig {
            schema,
            fragment_name,
            initialization_mode,
            resource_limits,
            scope,
        } = config;
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
            scope,
            schema_fingerprint,
            state: candidate.state,
            revision: 0,
            last_committed_origin: None,
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

    pub fn client_id(&self) -> u64 {
        self.doc.client_id()
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
        validate_import_document(&document, &self.schema, &self.resource_limits)?;

        let candidate = self.build_candidate_from_document(document, origin)?;
        Ok(self.commit_candidate(candidate, origin))
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
        validate_import_document(&document, &self.schema, &self.resource_limits)?;

        let candidate = self.build_candidate_from_document(document, origin)?;
        Ok(self.commit_candidate(candidate, origin))
    }

    fn build_candidate_from_document(
        &self,
        document: Document,
        origin: TransactionOrigin,
    ) -> YrsEngineResult<CandidateDocument> {
        let source_document = normalize_document_mark_order(&document);
        DocumentValidator::validate(&source_document, &self.schema, &self.resource_limits)
            .map_err(|error| {
                candidate_invariant_parse_error(error, "normalized source document is invalid")
            })?;
        let canonical_json = to_prosemirror_json(&source_document, &self.schema);
        let empty_json = json!({
            "type": self.schema.doc_node_type(),
            "content": [],
        });
        let doc = utf16_doc();
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
            codec.read_json(&fragment, &txn)?
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
        let canonical_json = to_prosemirror_json(&derived_document, &self.schema);

        Ok(CandidateDocument {
            doc,
            state: EngineDocumentState::Ready {
                document: derived_document,
                canonical_json,
            },
        })
    }

    fn commit_candidate(
        &mut self,
        candidate: CandidateDocument,
        origin: TransactionOrigin,
    ) -> EngineCommit {
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
            return EngineCommit {
                changed: false,
                revision: self.revision,
            };
        }

        self.doc = candidate.doc;
        self.state = candidate.state;
        self.revision = self.revision.saturating_add(1);
        self.last_committed_origin = Some(origin);
        EngineCommit {
            changed: true,
            revision: self.revision,
        }
    }
}

fn normalize_document_mark_order(document: &Document) -> Document {
    Document::new(normalize_node_mark_order(document.root()))
}

fn normalize_node_mark_order(node: &Node) -> Node {
    if node.is_text() {
        let mut marks = node.marks().to_vec();
        marks.sort_by(|left, right| left.mark_type().cmp(right.mark_type()));
        return Node::text(node.text_str().unwrap_or_default().to_string(), marks);
    }
    if node.is_void() {
        return Node::void(node.node_type().to_string(), node.attrs().clone());
    }
    let children = node
        .content()
        .into_iter()
        .flat_map(Fragment::iter)
        .map(normalize_node_mark_order)
        .collect();
    Node::element(
        node.node_type().to_string(),
        node.attrs().clone(),
        Fragment::from(children),
    )
}

fn rehydrate_reserved_html_opaque(document: &Document) -> Document {
    Document::new(rehydrate_reserved_html_opaque_node(document.root()))
}

fn rehydrate_reserved_html_opaque_node(node: &Node) -> Node {
    if let Some(attrs) = reserved_html_opaque_attrs(node) {
        return Node::void("__opaque".to_string(), attrs);
    }
    if node.is_text() {
        return Node::text(
            node.text_str().unwrap_or_default().to_string(),
            node.marks().to_vec(),
        );
    }
    if node.is_void() {
        return Node::void(node.node_type().to_string(), node.attrs().clone());
    }
    let children = node
        .content()
        .into_iter()
        .flat_map(Fragment::iter)
        .map(rehydrate_reserved_html_opaque_node)
        .collect();
    Node::element(
        node.node_type().to_string(),
        node.attrs().clone(),
        Fragment::from(children),
    )
}

fn reserved_html_opaque_attrs(
    node: &Node,
) -> Option<std::collections::HashMap<String, serde_json::Value>> {
    if node.node_type() != "__opaque_json" {
        return None;
    }
    let original = node.attrs().get("original_json")?.as_object()?;
    if original.get("type")?.as_str()? != "__opaque" {
        return None;
    }
    let attrs = original.get("attrs")?.as_object()?;
    Some(
        attrs
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
    )
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

    Ok(CandidateDocument {
        doc,
        state: EngineDocumentState::Ready {
            document,
            canonical_json,
        },
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
    })
}

fn utf16_doc() -> Doc {
    let options = Options {
        offset_kind: OffsetKind::Utf16,
        ..Options::default()
    };
    Doc::with_options(options)
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
    use yrs::OffsetKind;

    use super::utf16_doc;

    #[test]
    fn utf16_doc_preserves_fresh_client_ids_and_uses_utf16_offsets() {
        let first = utf16_doc();
        let second = utf16_doc();

        assert_eq!(first.offset_kind(), OffsetKind::Utf16);
        assert_eq!(second.offset_kind(), OffsetKind::Utf16);
        assert_ne!(first.client_id(), second.client_id());
    }
}
