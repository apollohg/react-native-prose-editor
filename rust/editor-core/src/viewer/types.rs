use std::sync::Arc;

use crate::ffi_v2::types::FfiError;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum FfiViewerSourceKind {
    Json,
    Html,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FfiViewerCompileRequest {
    pub source_kind: FfiViewerSourceKind,
    pub source: String,
    pub config_json: String,
    pub images_enabled: bool,
    pub mention_prefix: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FfiViewerMark {
    pub mark_type: String,
    pub attrs_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum FfiViewerElement {
    TextRun {
        text: String,
        marks: Vec<FfiViewerMark>,
    },
    InlineAtom {
        node_type: String,
        doc_pos: u32,
        attrs_json: String,
        label: String,
    },
    BlockAtom {
        node_type: String,
        doc_pos: u32,
        attrs_json: String,
        label: String,
    },
    BlockStart {
        node_type: String,
        depth: u16,
        list_context_json: Option<String>,
    },
    BlockEnd,
}

#[derive(uniffi::Object)]
pub struct ViewerCompiledDocument {
    pub(crate) semantic_key: String,
    pub(crate) elements: Vec<FfiViewerElement>,
    pub(crate) is_empty: bool,
    pub(crate) retained_bytes: usize,
}

#[uniffi::export]
impl ViewerCompiledDocument {
    pub fn semantic_key(&self) -> String {
        self.semantic_key.clone()
    }

    pub fn elements(&self) -> Vec<FfiViewerElement> {
        self.elements.clone()
    }

    pub fn is_empty(&self) -> bool {
        self.is_empty
    }

    pub fn retained_bytes_decimal(&self) -> String {
        self.retained_bytes.to_string()
    }
}

#[derive(uniffi::Record)]
pub struct FfiViewerCompileResult {
    pub value: Option<Arc<ViewerCompiledDocument>>,
    pub error: Option<FfiError>,
}

impl FfiViewerCompileResult {
    pub(crate) fn ok(value: Arc<ViewerCompiledDocument>) -> Self {
        Self {
            value: Some(value),
            error: None,
        }
    }

    pub(crate) fn err(error: FfiError) -> Self {
        Self {
            value: None,
            error: Some(error),
        }
    }
}
