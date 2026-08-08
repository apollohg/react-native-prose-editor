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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentOrigin {
    NativeView,
    JsApi,
    RemoteCollaboration,
    History,
    Restore,
    Import,
}

impl DocumentOrigin {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NativeView => "nativeView",
            Self::JsApi => "jsApi",
            Self::RemoteCollaboration => "remoteCollaboration",
            Self::History => "history",
            Self::Restore => "restore",
            Self::Import => "import",
        }
    }
}

impl From<TransactionOrigin> for DocumentOrigin {
    fn from(origin: TransactionOrigin) -> Self {
        match origin {
            TransactionOrigin::LocalInput
            | TransactionOrigin::LocalCommand
            | TransactionOrigin::LocalApi => Self::JsApi,
            TransactionOrigin::UndoRedo => Self::History,
            TransactionOrigin::RemoteSync => Self::RemoteCollaboration,
            TransactionOrigin::SnapshotRestore => Self::Restore,
            TransactionOrigin::DocumentImport => Self::Import,
        }
    }
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
