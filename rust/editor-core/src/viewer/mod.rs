mod compile;
mod types;

pub use types::{
    FfiViewerCompileRequest, FfiViewerCompileResult, FfiViewerElement, FfiViewerMark,
    FfiViewerSourceKind, ViewerCompiledDocument,
};

#[uniffi::export]
pub fn viewer_compile(request: FfiViewerCompileRequest) -> FfiViewerCompileResult {
    compile::compile(request)
}

#[cfg(test)]
mod tests;
