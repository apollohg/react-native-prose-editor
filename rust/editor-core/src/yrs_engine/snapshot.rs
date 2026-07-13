pub const SNAPSHOT_FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DocumentScope {
    pub document_id: String,
    pub lineage_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DocumentSnapshot {
    pub format_version: u32,
    pub document_id: String,
    pub lineage_id: String,
    pub fragment_name: String,
    pub schema_fingerprint: String,
    pub encoded_state: Vec<u8>,
}

impl DocumentSnapshot {
    pub(crate) fn metadata_byte_len(&self) -> usize {
        self.document_id
            .len()
            .saturating_add(self.lineage_id.len())
            .saturating_add(self.fragment_name.len())
            .saturating_add(self.schema_fingerprint.len())
    }
}

#[cfg(test)]
mod tests {
    use super::{DocumentSnapshot, SNAPSHOT_FORMAT_VERSION};

    #[test]
    fn snapshot_envelope_uses_the_approved_fields() {
        let snapshot = DocumentSnapshot {
            format_version: SNAPSHOT_FORMAT_VERSION,
            document_id: "document-1".into(),
            lineage_id: "lineage-2".into(),
            fragment_name: "prosemirror".into(),
            schema_fingerprint: "abc123".into(),
            encoded_state: vec![1, 2, 3],
        };
        assert_eq!(
            serde_json::to_value(snapshot).unwrap(),
            serde_json::json!({
                "formatVersion": 1,
                "documentId": "document-1",
                "lineageId": "lineage-2",
                "fragmentName": "prosemirror",
                "schemaFingerprint": "abc123",
                "encodedState": [1, 2, 3]
            })
        );
    }
}
