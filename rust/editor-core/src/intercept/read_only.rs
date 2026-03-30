use crate::intercept::{InterceptError, Interceptor};
use crate::model::Document;
use crate::transform::{Source, Transaction};

/// When locked, rejects all transactions except those from `Source::Api`.
///
/// When `locked` is `false`, this interceptor is a no-op pass-through.
pub struct ReadOnly {
    locked: bool,
}

/// ReadOnly applies to all sources (filtering happens inside `intercept`).
const READ_ONLY_SOURCES: &[Source] = &[
    Source::Input,
    Source::Format,
    Source::Paste,
    Source::History,
    Source::Api,
    Source::Reconciliation,
];

impl ReadOnly {
    /// Create a new ReadOnly interceptor.
    /// When `locked` is `true`, only `Source::Api` transactions pass through.
    pub fn new(locked: bool) -> Self {
        Self { locked }
    }

    /// Update the locked state at runtime.
    pub fn set_locked(&mut self, locked: bool) {
        self.locked = locked;
    }
}

impl Interceptor for ReadOnly {
    fn intercept(&self, tx: Transaction, _doc: &Document) -> Result<Transaction, InterceptError> {
        if !self.locked {
            return Ok(tx);
        }

        if tx.source == Source::Api {
            return Ok(tx);
        }

        Err(InterceptError::new(
            "document is read-only; only Api source transactions are allowed",
        ))
    }

    fn sources(&self) -> &[Source] {
        READ_ONLY_SOURCES
    }
}
