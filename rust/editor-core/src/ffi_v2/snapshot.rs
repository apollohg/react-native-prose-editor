//! UniFFI v2 snapshot entry points.
//!
//! Export returns the five-field manifest as JSON plus the encoded state as
//! direct bytes; restore takes the same two halves separately, so binary
//! state never appears inside JSON. Restore runs the session policy gate
//! (transport, outbox, manifest validation before decode).

#![allow(
    clippy::result_large_err,
    reason = "SessionError is the established unboxed session error envelope"
)]

use crate::boundary::{BoundaryError, BoundedInput, InputKind, ResourceLimits};
use crate::session::SessionError;
use crate::yrs_engine::DocumentSnapshot;

use super::editor::{json_result, with_editor, INTERNAL_UNCORRELATED_REQUEST_ID};
use super::types::{decimal_u64, FfiJsonResult, FfiSnapshotExport, FfiSnapshotExportResult};

/// The five snapshot manifest fields, wire-named; the encoded state rides
/// separately as direct bytes.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SnapshotMetadataEnvelope {
    pub(crate) format_version: u32,
    pub(crate) document_id: String,
    pub(crate) lineage_id: String,
    pub(crate) fragment_name: String,
    pub(crate) schema_fingerprint: String,
}

impl SnapshotMetadataEnvelope {
    pub(crate) fn into_snapshot(self, encoded_state: Vec<u8>) -> DocumentSnapshot {
        DocumentSnapshot {
            format_version: self.format_version,
            document_id: self.document_id,
            lineage_id: self.lineage_id,
            fragment_name: self.fragment_name,
            schema_fingerprint: self.schema_fingerprint,
            encoded_state,
        }
    }
}

fn metadata_json(snapshot: &DocumentSnapshot) -> String {
    serde_json::json!({
        "formatVersion": snapshot.format_version,
        "documentId": snapshot.document_id,
        "lineageId": snapshot.lineage_id,
        "fragmentName": snapshot.fragment_name,
        "schemaFingerprint": snapshot.schema_fingerprint,
    })
    .to_string()
}

#[uniffi::export]
pub fn editor_v2_snapshot_export(editor_id: String) -> FfiSnapshotExportResult {
    match with_editor(&editor_id, |session| {
        session
            .export_snapshot(INTERNAL_UNCORRELATED_REQUEST_ID)
            .map(|snapshot| FfiSnapshotExport {
                metadata_json: metadata_json(&snapshot),
                encoded_state: snapshot.encoded_state.clone(),
            })
    }) {
        Ok(export) => FfiSnapshotExportResult::ok(export),
        Err(error) => FfiSnapshotExportResult::err(error),
    }
}

#[uniffi::export]
pub fn editor_v2_snapshot_restore(
    editor_id: String,
    metadata_json: String,
    encoded_state: Vec<u8>,
) -> FfiJsonResult {
    json_result(with_editor(&editor_id, |session| {
        let input = BoundedInput::new(
            &metadata_json,
            InputKind::Config,
            &ResourceLimits::default(),
        )?;
        let metadata: SnapshotMetadataEnvelope = serde_json::from_str(input.as_str())
            .map_err(|error| SessionError::from(BoundaryError::parse("CONFIG_INVALID", error)))?;
        let snapshot = metadata.into_snapshot(encoded_state);
        session
            .restore_snapshot(INTERNAL_UNCORRELATED_REQUEST_ID, &snapshot)
            .map(|commit| {
                serde_json::json!({
                    "changed": commit.changed,
                    "documentRevision": decimal_u64(commit.revision),
                })
                .to_string()
            })
    }))
}
