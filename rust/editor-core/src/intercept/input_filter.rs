use regex::Regex;

use crate::intercept::{InterceptError, Interceptor};
use crate::model::Document;
use crate::transform::{Source, Step, Transaction};

/// Filters `InsertText` steps against a regex pattern, keeping only characters
/// that match. If all characters are removed, the step is dropped entirely.
///
/// Applies to sources: Input, Paste.
pub struct InputFilter {
    pattern: Regex,
}

/// Sources that InputFilter applies to.
const INPUT_FILTER_SOURCES: &[Source] = &[Source::Input, Source::Paste];

impl InputFilter {
    /// Create a new InputFilter with the given regex pattern.
    ///
    /// The pattern is matched per-character: each character in the inserted
    /// text is kept only if it matches the pattern.
    pub fn new(pattern: &str) -> Result<Self, regex::Error> {
        let regex = Regex::new(pattern)?;
        Ok(Self { pattern: regex })
    }
}

impl Interceptor for InputFilter {
    fn intercept(&self, tx: Transaction, _doc: &Document) -> Result<Transaction, InterceptError> {
        let mut filtered_steps: Vec<Step> = Vec::with_capacity(tx.steps.len());

        for step in tx.steps {
            match step {
                Step::InsertText { pos, text, marks } => {
                    let filtered: String = text
                        .chars()
                        .filter(|c| self.pattern.is_match(&c.to_string()))
                        .collect();

                    if !filtered.is_empty() {
                        filtered_steps.push(Step::InsertText {
                            pos,
                            text: filtered,
                            marks,
                        });
                    }
                    // If filtered is empty, the step is dropped (not added)
                }
                other => {
                    // Non-InsertText steps pass through unchanged
                    filtered_steps.push(other);
                }
            }
        }

        Ok(Transaction {
            steps: filtered_steps,
            source: tx.source,
            meta: tx.meta,
        })
    }

    fn sources(&self) -> &[Source] {
        INPUT_FILTER_SOURCES
    }
}
