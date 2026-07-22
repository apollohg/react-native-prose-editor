use std::sync::{Arc, OnceLock};

use sha2::{Digest, Sha256};

use crate::boundary::{serialize_json_value_stack_safe, StackSafeJsonValue};
use crate::model::Document;
use crate::schema::{schema_fingerprint, Schema};
use crate::serialize::to_prosemirror_json;

pub(crate) const CANONICAL_ARTIFACT_FORMAT_VERSION: u8 = 1;

const MIN_CANONICAL_JSON_INITIAL_CAPACITY: usize = 128;
const SMALL_CANONICAL_JSON_INITIAL_CAPACITY: usize = 64 * 1024;
const LARGE_CANONICAL_JSON_THRESHOLD: usize = 128 * 1024;
const LARGE_CANONICAL_JSON_INITIAL_CAPACITY: usize = 96 * 1024;

fn bounded_canonical_json_initial_capacity(admitted_upper_bound: usize) -> usize {
    if admitted_upper_bound == usize::MAX {
        return MIN_CANONICAL_JSON_INITIAL_CAPACITY;
    }
    let maximum = if admitted_upper_bound <= LARGE_CANONICAL_JSON_THRESHOLD {
        SMALL_CANONICAL_JSON_INITIAL_CAPACITY
    } else {
        LARGE_CANONICAL_JSON_INITIAL_CAPACITY
    };
    admitted_upper_bound.clamp(MIN_CANONICAL_JSON_INITIAL_CAPACITY, maximum)
}

fn serialize_canonical_json_with_hint(
    value: &serde_json::Value,
    admitted_upper_bound: usize,
) -> Vec<u8> {
    serialize_json_value_stack_safe(
        value,
        bounded_canonical_json_initial_capacity(admitted_upper_bound),
    )
}

#[derive(Clone, Debug)]
pub(crate) struct CanonicalSchemaContext(Arc<CanonicalSchemaContextInner>);

#[derive(Debug)]
struct CanonicalSchemaContextInner {
    schema: Arc<Schema>,
    schema_fingerprint: Arc<str>,
    format_version: u8,
}

impl CanonicalSchemaContext {
    pub(crate) fn new(schema: &Schema) -> Self {
        #[cfg(test)]
        SCHEMA_CONTEXT_COUNT.set(SCHEMA_CONTEXT_COUNT.get().saturating_add(1));
        Self(Arc::new(CanonicalSchemaContextInner {
            schema: Arc::new(schema.clone()),
            schema_fingerprint: schema_fingerprint(schema).into(),
            format_version: CANONICAL_ARTIFACT_FORMAT_VERSION,
        }))
    }

    pub(crate) fn derive(
        &self,
        document: &Document,
    ) -> Result<CanonicalArtifact, serde_json::Error> {
        CanonicalArtifact::derive_with_context(document, self)
    }

    pub(crate) fn derive_with_known_serialized_len(
        &self,
        document: &Document,
        serialized_len: usize,
    ) -> Result<CanonicalArtifact, serde_json::Error> {
        CanonicalArtifact::derive_with_context_and_admission(
            document,
            self,
            None,
            Some(serialized_len),
            Some(serialized_len),
        )
    }

    pub(crate) fn derive_with_known_text_metrics(
        &self,
        document: &Document,
        text_scalar_len: u64,
        text_utf8_bytes: usize,
    ) -> Result<CanonicalArtifact, serde_json::Error> {
        CanonicalArtifact::derive_with_context_and_text_metrics(
            document,
            self,
            Some((text_scalar_len, text_utf8_bytes)),
        )
    }

    pub(crate) fn derive_validated_json(
        &self,
        document: &Document,
        input_len: usize,
        _validation_work: usize,
    ) -> Result<CanonicalArtifact, serde_json::Error> {
        let admission_upper_bound =
            validated_json_admission_upper_bound(document, input_len, &self.0.schema)?;
        CanonicalArtifact::derive_with_context_and_admission(
            document,
            self,
            None,
            None,
            admission_upper_bound,
        )
    }

    pub(crate) fn schema_fingerprint(&self) -> &str {
        &self.0.schema_fingerprint
    }

    pub(crate) fn format_version(&self) -> u8 {
        self.0.format_version
    }

    pub(crate) fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    #[cfg(test)]
    pub(crate) fn schema(&self) -> &Schema {
        &self.0.schema
    }
}

fn validated_json_admission_upper_bound(
    document: &Document,
    input_len: usize,
    schema: &Schema,
) -> Result<Option<usize>, serde_json::Error> {
    let default_mark_expansions = schema
        .all_marks()
        .filter_map(|spec| {
            let defaults = spec
                .attrs
                .iter()
                .filter_map(|(name, attr)| {
                    attr.default.as_ref().map(|value| {
                        (
                            name.clone(),
                            crate::boundary::clone_json_value_stack_safe(value),
                        )
                    })
                })
                .collect::<serde_json::Map<_, _>>();
            (!defaults.is_empty()).then_some((spec.name.as_str(), defaults))
        })
        .map(|(name, defaults)| {
            let defaults = StackSafeJsonValue::new(serde_json::Value::Object(defaults));
            let serialized = serialize_json_value_stack_safe(defaults.as_value(), 0);
            // When input omits all mark attrs, canonical output adds this
            // fixed member syntax plus the exact compact default object.
            // Charging it for every occurrence remains conservative when
            // input supplied some or all attrs already.
            (name, b",\"attrs\":".len().saturating_add(serialized.len()))
        })
        .collect::<Vec<_>>();
    if default_mark_expansions.is_empty() {
        return Ok(Some(input_len));
    }

    let mut bound = input_len;
    let mut stack = vec![document.root()];
    while let Some(node) = stack.pop() {
        for mark in node.marks() {
            if let Some((_, expansion)) = default_mark_expansions
                .iter()
                .find(|(name, _)| *name == mark.mark_type())
            {
                let Some(next) = bound.checked_add(*expansion) else {
                    // An arithmetic proof is unavailable; force the exact
                    // canonical serialization fallback during admission.
                    return Ok(None);
                };
                bound = next;
            }
        }
        if let Some(content) = node.content() {
            stack.extend(content.iter());
        }
    }
    Ok(Some(bound))
}

#[derive(Debug)]
struct CanonicalArtifactInner {
    source_document: Document,
    value: StackSafeJsonValue,
    serialized_len: OnceLock<usize>,
    sha256: OnceLock<[u8; 32]>,
    admission_upper_bound: usize,
    text_scalar_len: u64,
    text_utf8_bytes: usize,
    schema_context: CanonicalSchemaContext,
}

#[derive(Clone, Debug)]
pub(crate) struct CanonicalArtifact(Arc<CanonicalArtifactInner>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CanonicalHistorySnapshotRetainedCharge {
    pub(crate) canonical_retained_bytes: usize,
    pub(crate) source_document_retained_bytes: usize,
}

#[derive(Debug)]
pub(crate) struct PreparedCanonicalCandidate {
    source_document: Document,
    value: StackSafeJsonValue,
    serialized_len: OnceLock<usize>,
    sha256: OnceLock<[u8; 32]>,
    sha256_provenance: OnceLock<[u8; 32]>,
    admission_upper_bound: usize,
    text_scalar_len: u64,
    text_utf8_bytes: usize,
    schema_context: CanonicalSchemaContext,
}

impl PartialEq for CanonicalArtifact {
    fn eq(&self, other: &Self) -> bool {
        self.ptr_eq(other)
            || crate::boundary::json_values_equal_stack_safe(self.value(), other.value())
    }
}

impl Eq for CanonicalArtifact {}

#[cfg(test)]
std::thread_local! {
    static DERIVATION_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static SERIALIZATION_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static SCHEMA_CONTEXT_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn reset_canonical_artifact_counts_for_test() {
    DERIVATION_COUNT.set(0);
    SERIALIZATION_COUNT.set(0);
}

#[cfg(test)]
pub(crate) fn take_canonical_artifact_counts_for_test() -> (usize, usize) {
    (DERIVATION_COUNT.replace(0), SERIALIZATION_COUNT.replace(0))
}

#[cfg(test)]
pub(crate) fn reset_canonical_schema_context_count_for_test() {
    SCHEMA_CONTEXT_COUNT.set(0);
}

#[cfg(test)]
pub(crate) fn take_canonical_schema_context_count_for_test() -> usize {
    SCHEMA_CONTEXT_COUNT.replace(0)
}

impl CanonicalArtifact {
    fn derive_with_context(
        document: &Document,
        schema_context: &CanonicalSchemaContext,
    ) -> Result<Self, serde_json::Error> {
        Self::derive_with_context_and_text_metrics(document, schema_context, None)
    }

    fn derive_with_context_and_text_metrics(
        document: &Document,
        schema_context: &CanonicalSchemaContext,
        known_text_metrics: Option<(u64, usize)>,
    ) -> Result<Self, serde_json::Error> {
        Self::derive_with_context_and_admission(
            document,
            schema_context,
            known_text_metrics,
            None,
            None,
        )
    }

    fn derive_with_context_and_admission(
        document: &Document,
        schema_context: &CanonicalSchemaContext,
        known_text_metrics: Option<(u64, usize)>,
        known_serialized_len: Option<usize>,
        admission_upper_bound: Option<usize>,
    ) -> Result<Self, serde_json::Error> {
        #[cfg(test)]
        DERIVATION_COUNT.set(DERIVATION_COUNT.get().saturating_add(1));

        #[cfg(test)]
        super::observability::record_canonical_projection();
        let value =
            StackSafeJsonValue::new(to_prosemirror_json(document, &schema_context.0.schema));
        let serialized_len = OnceLock::new();
        let exact_len = if let Some(len) = known_serialized_len {
            let _ = serialized_len.set(len);
            Some(len)
        } else if admission_upper_bound.is_none() {
            #[cfg(test)]
            super::observability::record_canonical_serialization();
            let len = serialize_json_value_stack_safe(value.as_value(), 0).len();
            let _ = serialized_len.set(len);
            #[cfg(test)]
            SERIALIZATION_COUNT.set(SERIALIZATION_COUNT.get().saturating_add(1));
            Some(len)
        } else {
            None
        };
        let (text_scalar_len, text_utf8_bytes) =
            known_text_metrics.unwrap_or_else(|| raw_text_metrics(document));
        Ok(Self(Arc::new(CanonicalArtifactInner {
            source_document: document.clone(),
            value,
            serialized_len,
            sha256: OnceLock::new(),
            admission_upper_bound: admission_upper_bound.or(exact_len).unwrap_or(usize::MAX),
            text_scalar_len,
            text_utf8_bytes,
            schema_context: schema_context.clone(),
        })))
    }

    pub(crate) fn value(&self) -> &serde_json::Value {
        self.0.value.as_value()
    }

    pub(crate) fn serialized_len(&self) -> usize {
        *self.0.serialized_len.get_or_init(|| {
            #[cfg(test)]
            super::observability::record_canonical_serialization();
            let len = serialize_json_value_stack_safe(self.0.value.as_value(), 0).len();
            #[cfg(test)]
            SERIALIZATION_COUNT.set(SERIALIZATION_COUNT.get().saturating_add(1));
            len
        })
    }

    pub(crate) fn sha256(&self) -> [u8; 32] {
        *self.0.sha256.get_or_init(|| {
            let serialized = serialize_canonical_json_with_hint(
                self.0.value.as_value(),
                self.0.admission_upper_bound,
            );
            #[cfg(test)]
            super::observability::record_canonical_serialization();
            #[cfg(test)]
            SERIALIZATION_COUNT.set(SERIALIZATION_COUNT.get().saturating_add(1));
            let _ = self.0.serialized_len.set(serialized.len());
            #[cfg(test)]
            super::observability::record_canonical_hash();
            canonical_sha256(&serialized)
        })
    }

    pub(crate) fn admitted_serialized_upper_bound(&self) -> usize {
        self.0.admission_upper_bound
    }

    pub(crate) fn admitted_serialized_upper_bound_option(&self) -> Option<usize> {
        (self.admitted_serialized_upper_bound() != usize::MAX)
            .then_some(self.admitted_serialized_upper_bound())
    }

    #[cfg(test)]
    pub(crate) fn with_admission_upper_bound_for_test(&self, admission_upper_bound: usize) -> Self {
        Self(Arc::new(CanonicalArtifactInner {
            source_document: self.0.source_document.clone(),
            value: self.0.value.clone(),
            serialized_len: self.0.serialized_len.clone(),
            sha256: self.0.sha256.clone(),
            admission_upper_bound,
            text_scalar_len: self.0.text_scalar_len,
            text_utf8_bytes: self.0.text_utf8_bytes,
            schema_context: self.0.schema_context.clone(),
        }))
    }

    pub(crate) fn text_scalar_len(&self) -> u64 {
        self.0.text_scalar_len
    }

    pub(crate) fn text_utf8_bytes(&self) -> usize {
        self.0.text_utf8_bytes
    }

    pub(crate) fn schema_fingerprint(&self) -> &str {
        self.0.schema_context.schema_fingerprint()
    }

    pub(crate) fn format_version(&self) -> u8 {
        self.0.schema_context.format_version()
    }

    pub(crate) fn schema_context(&self) -> &CanonicalSchemaContext {
        &self.0.schema_context
    }

    /// Proves that this artifact was derived from this exact document under
    /// its sealed schema context. Callers must not combine independently
    /// supplied documents and artifacts without this check.
    pub(crate) fn matches_document(&self, document: &Document) -> bool {
        #[cfg(test)]
        super::observability::record_canonical_projection();
        let value = StackSafeJsonValue::new(to_prosemirror_json(
            document,
            &self.0.schema_context.0.schema,
        ));
        #[cfg(test)]
        super::observability::record_canonical_serialization();
        let serialized = serialize_json_value_stack_safe(value.as_value(), 0);
        self.sha256() == canonical_sha256(&serialized)
            && self.serialized_len() == serialized.len()
            && crate::boundary::json_values_equal_stack_safe(self.value(), value.as_value())
    }

    pub(crate) fn matches_exact_source_document(&self, document: &Document) -> bool {
        self.0.source_document.shares_root_storage_with(document)
    }

    pub(crate) fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    pub(crate) fn history_snapshot_retained_bytes(&self) -> Option<usize> {
        self.history_snapshot_retained_charge()
            .map(|charge| charge.canonical_retained_bytes)
    }

    pub(crate) fn history_snapshot_retained_charge(
        &self,
    ) -> Option<CanonicalHistorySnapshotRetainedCharge> {
        // The engine permanently owns the canonical schema context. The source
        // document normally aliases the separately metered snapshot Document,
        // but counting it again keeps this helper conservative even if a future
        // caller loses that identity invariant.
        let source_document_retained_bytes =
            self.0.source_document.history_snapshot_retained_bytes()?;
        let canonical_retained_bytes = crate::model::arc_allocation_retained_bytes(
            std::mem::size_of::<CanonicalArtifactInner>(),
        )?
        .checked_add(source_document_retained_bytes)?
        .checked_add(crate::model::json_value_retained_bytes(
            self.0.value.as_value(),
        )?)?;
        Some(CanonicalHistorySnapshotRetainedCharge {
            canonical_retained_bytes,
            source_document_retained_bytes,
        })
    }
}

#[allow(dead_code)]
impl PreparedCanonicalCandidate {
    pub(crate) fn prepare(
        document: &Document,
        schema_context: &CanonicalSchemaContext,
        admission_upper_bound: usize,
    ) -> Self {
        #[cfg(test)]
        DERIVATION_COUNT.set(DERIVATION_COUNT.get().saturating_add(1));
        #[cfg(test)]
        super::observability::record_canonical_projection();
        let value =
            StackSafeJsonValue::new(to_prosemirror_json(document, &schema_context.0.schema));
        let (text_scalar_len, text_utf8_bytes) = raw_text_metrics(document);
        Self {
            source_document: document.clone(),
            value,
            serialized_len: OnceLock::new(),
            sha256: OnceLock::new(),
            sha256_provenance: OnceLock::new(),
            admission_upper_bound,
            text_scalar_len,
            text_utf8_bytes,
            schema_context: schema_context.clone(),
        }
    }

    pub(crate) fn serialized_len(&self) -> usize {
        *self.serialized_len.get_or_init(|| {
            #[cfg(test)]
            super::observability::record_canonical_serialization();
            let len = serialize_json_value_stack_safe(self.value.as_value(), 0).len();
            #[cfg(test)]
            SERIALIZATION_COUNT.set(SERIALIZATION_COUNT.get().saturating_add(1));
            len
        })
    }

    pub(crate) fn sha256(&self) -> [u8; 32] {
        *self.sha256.get_or_init(|| {
            let serialized = serialize_canonical_json_with_hint(
                self.value.as_value(),
                self.admission_upper_bound,
            );
            #[cfg(test)]
            super::observability::record_canonical_serialization();
            #[cfg(test)]
            SERIALIZATION_COUNT.set(SERIALIZATION_COUNT.get().saturating_add(1));
            let _ = self.serialized_len.set(serialized.len());
            #[cfg(test)]
            super::observability::record_canonical_hash();
            let sha256 = canonical_sha256(&serialized);
            let _ = self.sha256_provenance.set(sha256);
            sha256
        })
    }

    pub(crate) fn admission_upper_bound(&self) -> usize {
        self.admission_upper_bound
    }

    pub(crate) fn text_scalar_len(&self) -> u64 {
        self.text_scalar_len
    }

    pub(crate) fn text_utf8_bytes(&self) -> usize {
        self.text_utf8_bytes
    }

    pub(crate) fn exact_history_identity(&self) -> (usize, [u8; 32]) {
        // Hashing materializes the exact serialized bytes and seeds the length
        // cache in the same pass. Reading the cached length afterwards avoids
        // a second candidate serialization before deferred finalization.
        let sha256 = self.sha256();
        (self.serialized_len(), sha256)
    }

    pub(crate) fn matches_exact_source_document(&self, document: &Document) -> bool {
        self.source_document.shares_root_storage_with(document)
    }

    pub(crate) fn history_snapshot_retained_bytes(&self) -> Option<usize> {
        self.history_snapshot_retained_charge()
            .map(|charge| charge.canonical_retained_bytes)
    }

    pub(crate) fn history_snapshot_retained_charge(
        &self,
    ) -> Option<CanonicalHistorySnapshotRetainedCharge> {
        // Finalization moves these exact owned fields into a CanonicalArtifact.
        // Charge the future Arc payload now without copying or sealing it.
        let source_document_retained_bytes =
            self.source_document.history_snapshot_retained_bytes()?;
        let canonical_retained_bytes = crate::model::arc_allocation_retained_bytes(
            std::mem::size_of::<CanonicalArtifactInner>(),
        )?
        .checked_add(source_document_retained_bytes)?
        .checked_add(crate::model::json_value_retained_bytes(
            self.value.as_value(),
        )?)?;
        Some(CanonicalHistorySnapshotRetainedCharge {
            canonical_retained_bytes,
            source_document_retained_bytes,
        })
    }

    pub(crate) fn seal_with_known_serialized_len(
        self,
        exact_len: usize,
    ) -> Option<CanonicalArtifact> {
        if self
            .serialized_len
            .get()
            .is_some_and(|materialized| *materialized != exact_len)
            || !match (self.sha256.get(), self.sha256_provenance.get()) {
                (None, None) => true,
                (Some(sha256), Some(provenance)) => sha256 == provenance,
                _ => false,
            }
        {
            return None;
        }
        let _ = self.serialized_len.set(exact_len);
        Some(CanonicalArtifact(Arc::new(CanonicalArtifactInner {
            source_document: self.source_document,
            value: self.value,
            serialized_len: self.serialized_len,
            sha256: self.sha256,
            admission_upper_bound: self.admission_upper_bound,
            text_scalar_len: self.text_scalar_len,
            text_utf8_bytes: self.text_utf8_bytes,
            schema_context: self.schema_context,
        })))
    }

    #[cfg(test)]
    pub(crate) fn warm_scalar_caches_for_test(&self) -> (usize, [u8; 32]) {
        (self.serialized_len(), self.sha256())
    }

    #[cfg(test)]
    pub(crate) fn tamper_scalar_cache_for_test(&mut self, case: &str) {
        match case {
            "length" => {
                let materialized = self
                    .serialized_len
                    .take()
                    .expect("length tamper requires a warmed candidate cache");
                let _ = self.serialized_len.set(materialized.saturating_add(1));
            }
            "sha256" => {
                let mut materialized = self
                    .sha256
                    .take()
                    .expect("SHA tamper requires a warmed candidate cache");
                materialized[0] ^= 1;
                let _ = self.sha256.set(materialized);
            }
            _ => panic!("unknown prepared candidate cache tamper case {case}"),
        }
    }
}

fn canonical_sha256(bytes: &[u8]) -> [u8; 32] {
    #[cfg(target_vendor = "apple")]
    if let Ok(len) = u32::try_from(bytes.len()) {
        let mut output = [0_u8; 32];
        // SAFETY: `bytes` and `output` remain live for the complete call,
        // their pointers cover the supplied lengths, and CC_SHA256 writes
        // exactly 32 bytes to a non-null digest buffer.
        let digest = unsafe { CC_SHA256(bytes.as_ptr().cast(), len, output.as_mut_ptr()) };
        if !digest.is_null() {
            return output;
        }
    }
    Sha256::digest(bytes).into()
}

#[cfg(target_vendor = "apple")]
#[link(name = "System")]
unsafe extern "C" {
    fn CC_SHA256(data: *const std::ffi::c_void, len: u32, digest: *mut u8) -> *mut u8;
}

fn raw_text_metrics(document: &Document) -> (u64, usize) {
    #[cfg(test)]
    super::observability::record_raw_document_text_scan();
    let mut scalars = 0_u64;
    let mut utf8_bytes = 0_usize;
    let mut stack = vec![document.root()];
    while let Some(node) = stack.pop() {
        if let Some(text) = node.text_str() {
            scalars =
                scalars.saturating_add(u64::try_from(text.chars().count()).unwrap_or(u64::MAX));
            utf8_bytes = utf8_bytes.saturating_add(text.len());
        } else if let Some(content) = node.content() {
            stack.extend(content.iter());
        }
    }
    (scalars, utf8_bytes)
}

#[cfg(test)]
mod tests {
    use sha2::{Digest, Sha256};

    use super::{
        bounded_canonical_json_initial_capacity, canonical_sha256,
        reset_canonical_artifact_counts_for_test, serialize_canonical_json_with_hint,
        take_canonical_artifact_counts_for_test, CanonicalArtifactInner, CanonicalSchemaContext,
        PreparedCanonicalCandidate, CANONICAL_ARTIFACT_FORMAT_VERSION,
        LARGE_CANONICAL_JSON_INITIAL_CAPACITY, LARGE_CANONICAL_JSON_THRESHOLD,
        SMALL_CANONICAL_JSON_INITIAL_CAPACITY,
    };
    use crate::schema::{presets::tiptap_schema, schema_fingerprint};
    use crate::serialize::{from_prosemirror_json, to_prosemirror_json, UnknownTypeMode};

    #[test]
    fn artifact_metrics_are_from_the_exact_canonical_projection() {
        let schema = tiptap_schema();
        let context = CanonicalSchemaContext::new(&schema);
        let fingerprint = schema_fingerprint(&schema);
        let source = serde_json::json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [
                    {"type": "text", "text": "a🙂", "marks": [{"type": "bold"}]},
                    {"type": "hardBreak"},
                    {"type": "text", "text": "é"}
                ]
            }]
        });
        let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Error).unwrap();
        let expected_value = to_prosemirror_json(&document, &schema);
        let expected_bytes = serde_json::to_vec(&expected_value).unwrap();
        let expected_sha256: [u8; 32] = Sha256::digest(&expected_bytes).into();

        reset_canonical_artifact_counts_for_test();
        let artifact = context.derive(&document).unwrap();

        assert_eq!(artifact.value(), &expected_value);
        assert_eq!(artifact.serialized_len(), expected_bytes.len());
        assert_eq!(artifact.sha256(), expected_sha256);
        assert_eq!(artifact.text_scalar_len(), 4);
        assert_eq!(artifact.text_utf8_bytes(), 8);
        assert_eq!(artifact.schema_fingerprint(), fingerprint);
        assert_eq!(artifact.format_version(), CANONICAL_ARTIFACT_FORMAT_VERSION);
        assert!(context.ptr_eq(artifact.schema_context()));
        assert_eq!(take_canonical_artifact_counts_for_test(), (1, 2));
    }

    #[test]
    fn sha256_records_its_serialization_even_when_length_is_already_cached() {
        let schema = tiptap_schema();
        let context = CanonicalSchemaContext::new(&schema);
        let document = from_prosemirror_json(
            &serde_json::json!({
                "type": "doc",
                "content": [{
                    "type": "paragraph",
                    "content": [{"type": "text", "text": "cached length"}]
                }]
            }),
            &schema,
            UnknownTypeMode::Error,
        )
        .unwrap();
        let artifact = context.derive(&document).unwrap();
        let _ = artifact.serialized_len();
        crate::yrs_engine::observability::reset_full_pass_counts_for_test();

        let _ = artifact.sha256();

        let passes = crate::yrs_engine::observability::take_full_pass_counts_for_test();
        assert_eq!(passes.canonical_serializations, 1);
        assert_eq!(passes.canonical_hashes, 1);
        let _ = artifact.sha256();
        let cached = crate::yrs_engine::observability::take_full_pass_counts_for_test();
        assert_eq!(cached.canonical_serializations, 0);
        assert_eq!(cached.canonical_hashes, 0);
    }

    #[test]
    fn platform_canonical_sha256_matches_portable_sha2_for_fixed_and_random_bytes() {
        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        let mut samples = vec![
            Vec::new(),
            vec![0],
            vec![0xff; 31],
            vec![0x55; 32],
            vec![0xaa; 55],
            vec![0x11; 56],
            vec![0x22; 63],
            vec![0x33; 64],
            vec![0x44; 65],
            vec![0x78; 256 * 1024],
            vec![0x79; 512 * 1024],
        ];
        for _ in 0..64 {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let len = usize::try_from(state % 16_385).unwrap();
            let mut bytes = Vec::with_capacity(len);
            for _ in 0..len {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                bytes.push(state.to_le_bytes()[0]);
            }
            samples.push(bytes);
        }

        for bytes in samples {
            assert_eq!(
                canonical_sha256(&bytes),
                <[u8; 32]>::from(Sha256::digest(&bytes)),
                "length {}",
                bytes.len()
            );
        }
    }

    #[test]
    fn bounded_canonical_json_serialization_matches_serde_json_exactly() {
        let values = [
            serde_json::json!(null),
            serde_json::json!({}),
            serde_json::json!([]),
            serde_json::json!({
                "type": "doc",
                "content": [{
                    "type": "paragraph",
                    "attrs": {
                        "empty": "",
                        "escaped": "quote=\" slash=\\ controls=\n\r\t",
                        "unicode": "é🙂雪"
                    },
                    "content": [
                        {"type": "text", "text": "plain"},
                        {"type": "text", "text": "x".repeat(144 * 1024)}
                    ]
                }],
                "numbers": [0, -1, 9223372036854775807_i64, 0.0, -12.5, 6.022e23]
            }),
        ];

        for value in values {
            let expected = serde_json::to_vec(&value).unwrap();
            let actual = serialize_canonical_json_with_hint(&value, expected.len());

            assert_eq!(actual, expected);
            assert_eq!(canonical_sha256(&actual), canonical_sha256(&expected));
        }
    }

    #[test]
    fn canonical_json_initial_capacity_is_strictly_bounded() {
        assert_eq!(bounded_canonical_json_initial_capacity(0), 128);
        assert_eq!(bounded_canonical_json_initial_capacity(127), 128);
        assert_eq!(bounded_canonical_json_initial_capacity(128), 128);
        assert_eq!(bounded_canonical_json_initial_capacity(4_096), 4_096);
        assert_eq!(
            bounded_canonical_json_initial_capacity(SMALL_CANONICAL_JSON_INITIAL_CAPACITY),
            SMALL_CANONICAL_JSON_INITIAL_CAPACITY
        );
        assert_eq!(
            bounded_canonical_json_initial_capacity(LARGE_CANONICAL_JSON_THRESHOLD),
            SMALL_CANONICAL_JSON_INITIAL_CAPACITY
        );
        assert_eq!(
            bounded_canonical_json_initial_capacity(
                LARGE_CANONICAL_JSON_THRESHOLD.saturating_add(1)
            ),
            LARGE_CANONICAL_JSON_INITIAL_CAPACITY
        );
        assert_eq!(
            bounded_canonical_json_initial_capacity(
                LARGE_CANONICAL_JSON_THRESHOLD
                    .saturating_add(LARGE_CANONICAL_JSON_INITIAL_CAPACITY)
            ),
            LARGE_CANONICAL_JSON_INITIAL_CAPACITY
        );
        assert_eq!(
            bounded_canonical_json_initial_capacity(usize::MAX.saturating_sub(1)),
            LARGE_CANONICAL_JSON_INITIAL_CAPACITY
        );
        assert_eq!(bounded_canonical_json_initial_capacity(usize::MAX), 128);
    }

    #[test]
    fn cloning_an_artifact_only_clones_its_arc_handle() {
        let schema = tiptap_schema();
        let context = CanonicalSchemaContext::new(&schema);
        let source = serde_json::json!({
            "type": "doc",
            "content": [{"type": "paragraph", "content": [{"type": "text", "text": "x"}]}]
        });
        let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Error).unwrap();
        let artifact = context.derive(&document).unwrap();

        assert!(artifact.ptr_eq(&artifact.clone()));
    }

    #[test]
    fn artifact_identity_rejects_a_same_schema_document_swap() {
        let schema = tiptap_schema();
        let context = CanonicalSchemaContext::new(&schema);
        let left = from_prosemirror_json(
            &serde_json::json!({
                "type": "doc",
                "content": [{"type": "paragraph", "content": [{"type": "text", "text": "left"}]}]
            }),
            &schema,
            UnknownTypeMode::Error,
        )
        .unwrap();
        let right = from_prosemirror_json(
            &serde_json::json!({
                "type": "doc",
                "content": [{"type": "paragraph", "content": [{"type": "text", "text": "right"}]}]
            }),
            &schema,
            UnknownTypeMode::Error,
        )
        .unwrap();
        let artifact = context.derive(&left).unwrap();

        assert!(artifact.matches_document(&left));
        assert!(!artifact.matches_document(&right));
    }

    #[test]
    fn history_retention_charge_preserves_the_legacy_exact_total() {
        let schema = tiptap_schema();
        let context = CanonicalSchemaContext::new(&schema);
        let document = from_prosemirror_json(
            &serde_json::json!({
                "type": "doc",
                "content": [{
                    "type": "paragraph",
                    "content": [{"type": "text", "text": "retained"}]
                }]
            }),
            &schema,
            UnknownTypeMode::Error,
        )
        .unwrap();
        let artifact = context.derive(&document).unwrap();
        let prepared = PreparedCanonicalCandidate::prepare(&document, &context, usize::MAX);
        let legacy = crate::model::arc_allocation_retained_bytes(std::mem::size_of::<
            CanonicalArtifactInner,
        >())
        .unwrap()
        .checked_add(document.history_snapshot_retained_bytes().unwrap())
        .unwrap()
        .checked_add(crate::model::json_value_retained_bytes(artifact.value()).unwrap())
        .unwrap();

        let artifact_charge = artifact.history_snapshot_retained_charge().unwrap();
        let prepared_charge = prepared.history_snapshot_retained_charge().unwrap();

        assert_eq!(artifact_charge.canonical_retained_bytes, legacy);
        assert_eq!(prepared_charge.canonical_retained_bytes, legacy);
        assert_eq!(
            artifact_charge.source_document_retained_bytes,
            document.history_snapshot_retained_bytes().unwrap()
        );
        assert_eq!(prepared_charge, artifact_charge);
        assert_eq!(artifact.history_snapshot_retained_bytes(), Some(legacy));
        assert_eq!(prepared.history_snapshot_retained_bytes(), Some(legacy));
    }
}
