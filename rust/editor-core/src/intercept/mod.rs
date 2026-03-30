pub mod input_filter;
pub mod max_length;
pub mod read_only;

pub use input_filter::InputFilter;
pub use max_length::MaxLength;
pub use read_only::ReadOnly;

use crate::model::Document;
use crate::transform::{Source, Transaction};

// ---------------------------------------------------------------------------
// InterceptError
// ---------------------------------------------------------------------------

/// Error returned when an interceptor rejects a transaction.
#[derive(Debug)]
pub struct InterceptError {
    pub message: String,
}

impl InterceptError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for InterceptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "intercept error: {}", self.message)
    }
}

// ---------------------------------------------------------------------------
// Interceptor trait
// ---------------------------------------------------------------------------

/// A pre-commit interceptor that can inspect and modify or reject a transaction.
///
/// Interceptors are parameterized built-ins configured at editor init time.
/// They run before a transaction commits and can:
/// - Pass the transaction through unchanged
/// - Modify the transaction (e.g., filter input characters)
/// - Reject the transaction entirely
pub trait Interceptor: Send + Sync {
    /// Called before a transaction commits.
    /// Return `Ok(tx)` to proceed (possibly modified), `Err` to abort.
    fn intercept(&self, tx: Transaction, doc: &Document) -> Result<Transaction, InterceptError>;

    /// Which transaction sources this interceptor applies to.
    /// If the transaction's source is not in this list, the interceptor is skipped.
    fn sources(&self) -> &[Source];
}

/// Extension method for interceptors: checks source applicability, then delegates
/// to `intercept`. This is the public entry point callers should use.
pub trait InterceptorExt: Interceptor {
    fn check(&self, tx: Transaction, doc: &Document) -> Result<Transaction, InterceptError>;
}

impl<T: ?Sized + Interceptor> InterceptorExt for T {
    fn check(&self, tx: Transaction, doc: &Document) -> Result<Transaction, InterceptError> {
        if !self.sources().contains(&tx.source) {
            return Ok(tx);
        }
        self.intercept(tx, doc)
    }
}

// ---------------------------------------------------------------------------
// InterceptorPipeline
// ---------------------------------------------------------------------------

/// Runs a sequence of interceptors in order against a transaction.
///
/// Each interceptor may modify the transaction before passing it to the next.
/// If any interceptor returns `Err`, the pipeline aborts immediately.
pub struct InterceptorPipeline {
    interceptors: Vec<Box<dyn Interceptor>>,
}

impl InterceptorPipeline {
    /// Create an empty pipeline with no interceptors.
    pub fn new() -> Self {
        Self {
            interceptors: Vec::new(),
        }
    }

    /// Append an interceptor to the end of the pipeline.
    pub fn add(&mut self, interceptor: Box<dyn Interceptor>) {
        self.interceptors.push(interceptor);
    }

    /// Run all interceptors in order. If any returns `Err`, the transaction is
    /// aborted and the error is returned. Only runs interceptors whose
    /// `sources()` include the transaction's source.
    pub fn run(&self, tx: Transaction, doc: &Document) -> Result<Transaction, InterceptError> {
        let mut current = tx;
        for interceptor in &self.interceptors {
            current = interceptor.check(current, doc)?;
        }
        Ok(current)
    }
}

impl Default for InterceptorPipeline {
    fn default() -> Self {
        Self::new()
    }
}
