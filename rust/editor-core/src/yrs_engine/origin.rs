#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TransactionOrigin {
    LocalInput,
    LocalCommand,
    LocalApi,
    UndoRedo,
    RemoteSync,
    SnapshotRestore,
    DocumentImport,
}

impl TransactionOrigin {
    pub const fn as_tag(self) -> &'static str {
        match self {
            Self::LocalInput => "native-editor/local-input",
            Self::LocalCommand => "native-editor/local-command",
            Self::LocalApi => "native-editor/local-api",
            Self::UndoRedo => "native-editor/undo-redo",
            Self::RemoteSync => "native-editor/remote-sync",
            Self::SnapshotRestore => "native-editor/snapshot-restore",
            Self::DocumentImport => "native-editor/document-import",
        }
    }

    pub fn as_yrs_origin(self) -> yrs::Origin {
        yrs::Origin::from(self.as_tag())
    }
}

#[cfg(test)]
mod tests {
    use super::TransactionOrigin;

    #[test]
    fn transaction_origins_have_stable_yrs_tags() {
        let cases = [
            (TransactionOrigin::LocalInput, "native-editor/local-input"),
            (
                TransactionOrigin::LocalCommand,
                "native-editor/local-command",
            ),
            (TransactionOrigin::LocalApi, "native-editor/local-api"),
            (TransactionOrigin::UndoRedo, "native-editor/undo-redo"),
            (TransactionOrigin::RemoteSync, "native-editor/remote-sync"),
            (
                TransactionOrigin::SnapshotRestore,
                "native-editor/snapshot-restore",
            ),
            (
                TransactionOrigin::DocumentImport,
                "native-editor/document-import",
            ),
        ];
        for (origin, expected) in cases {
            assert_eq!(origin.as_tag(), expected);
            assert_eq!(origin.as_yrs_origin().as_ref(), expected.as_bytes());
        }
    }
}
